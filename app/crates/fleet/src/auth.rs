//! Who is asking: a person signed in to the console, or a joined computer.

use crate::store::{random_token, sha256_hex, Person, Store};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ROUNDS: u32 = 210_000;
const SESSION: Duration = Duration::from_secs(12 * 3600);

pub fn hash_password(password: &str, salt_hex: &str) -> String {
    let mut out = [0u8; 32];
    ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(ROUNDS).unwrap(), salt_hex.as_bytes(), password.as_bytes(), &mut out);
    hex::encode(out)
}

pub fn new_person(name: &str, role: &str, password: &str) -> Result<Person, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c)) {
        return Err("A name can use letters, digits and . _ - @".into());
    }
    if !valid_role(role) {
        return Err("The role is one of admin, auditor, viewer.".into());
    }
    if password.chars().count() < 10 {
        return Err("Choose a password of at least ten characters.".into());
    }
    let salt = random_token(16);
    Ok(Person { name: name.to_string(), role: role.to_string(), hash: hash_password(password, &salt), salt, active: true })
}

pub fn valid_role(role: &str) -> bool {
    matches!(role, "admin" | "auditor" | "viewer")
}

/// What a role may do. Each level includes the ones before it.
pub fn allows(role: &str, need: &str) -> bool {
    let rank = |r: &str| match r {
        "admin" => 3,
        "auditor" => 2,
        "viewer" => 1,
        _ => 0,
    };
    rank(role) >= rank(need) && rank(need) > 0
}

#[derive(Default)]
pub struct Sessions {
    live: Mutex<HashMap<String, (String, Instant)>>,
    /// Failed sign-ins per name, to slow guessing down.
    failed: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Sessions {
    /// Check a name and password. On success, a new session's secret.
    pub fn sign_in(&self, store: &Store, name: &str, password: &str) -> Result<String, String> {
        let key = name.trim().to_lowercase();
        {
            let failed = self.failed.lock().unwrap();
            if let Some((n, since)) = failed.get(&key) {
                if *n >= 8 && since.elapsed() < Duration::from_secs(300) {
                    return Err("Too many wrong passwords. Wait five minutes and try again.".into());
                }
            }
        }
        let person = store.people.lock().unwrap().iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()) && p.active).cloned();
        // The same work whether or not the name exists, so the answer's timing says nothing.
        let (salt, expected) = person.as_ref().map(|p| (p.salt.clone(), p.hash.clone())).unwrap_or_else(|| ("no-such-person".into(), String::new()));
        let got = hash_password(password, &salt);
        let ok = person.is_some() && crate::store::same(&got, &expected);
        if !ok {
            let mut failed = self.failed.lock().unwrap();
            let e = failed.entry(key).or_insert((0, Instant::now()));
            if e.1.elapsed() > Duration::from_secs(300) {
                *e = (0, Instant::now());
            }
            e.0 += 1;
            return Err("That name and password do not match.".into());
        }
        self.failed.lock().unwrap().remove(&key);
        Ok(self.start(&person.unwrap().name))
    }

    /// Begin a session for someone already checked (a password, or the identity provider).
    pub fn start(&self, name: &str) -> String {
        let secret = random_token(32);
        let mut live = self.live.lock().unwrap();
        live.retain(|_, (_, at)| at.elapsed() < SESSION);
        live.insert(sha256_hex(&secret), (name.to_string(), Instant::now()));
        secret
    }

    /// The person behind a session cookie, if the session is live and they are still active.
    pub fn person(&self, store: &Store, secret: &str) -> Option<Person> {
        let name = {
            let live = self.live.lock().unwrap();
            let (name, at) = live.get(&sha256_hex(secret))?;
            (at.elapsed() < SESSION).then(|| name.clone())?
        };
        store.people.lock().unwrap().iter().find(|p| p.name == name && p.active).cloned()
    }

    pub fn sign_out(&self, secret: &str) {
        self.live.lock().unwrap().remove(&sha256_hex(secret));
    }

    /// End every session of one person (their role changed, or they were switched off).
    pub fn end_all(&self, name: &str) {
        self.live.lock().unwrap().retain(|_, (n, _)| n != name);
    }
}

/// The joined computer behind `Authorization: Bearer <id>.<secret>`.
pub fn device(store: &Store, header: Option<&str>) -> Option<String> {
    let (id, secret) = header?.strip_prefix("Bearer ")?.split_once('.')?;
    let devices = store.devices.lock().unwrap();
    let d = devices.get(id)?;
    crate::store::same(&sha256_hex(secret), &d.secret_hash).then_some(())?;
    Some(d.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_include_the_ones_below_them() {
        assert!(allows("admin", "auditor") && allows("admin", "viewer") && allows("auditor", "viewer"));
        assert!(!allows("viewer", "auditor") && !allows("auditor", "admin") && !allows("nobody", "viewer") && !allows("admin", "nonsense"));
    }

    #[test]
    fn people_need_a_plain_name_a_known_role_and_a_real_password() {
        assert!(new_person("dana", "auditor", "long enough pw").is_ok());
        assert!(new_person("dana smith", "auditor", "long enough pw").is_err());
        assert!(new_person("dana", "owner", "long enough pw").is_err());
        assert!(new_person("dana", "admin", "short").is_err());
    }

    #[test]
    fn signing_in_needs_the_right_password_and_guessing_is_slowed() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path(), "Org").unwrap();
        store.people.lock().unwrap().push(new_person("dana", "admin", "correct horse battery").unwrap());
        let s = Sessions::default();
        assert!(s.sign_in(&store, "dana", "wrong password").is_err());
        assert!(s.sign_in(&store, "nobody", "correct horse battery").is_err());
        let secret = s.sign_in(&store, "Dana", "correct horse battery").unwrap();
        assert_eq!(s.person(&store, &secret).unwrap().role, "admin");
        assert!(s.person(&store, "made-up").is_none());
        // Switched off: the open session stops working at once.
        store.people.lock().unwrap()[0].active = false;
        assert!(s.person(&store, &secret).is_none());
        store.people.lock().unwrap()[0].active = true;
        for _ in 0..8 {
            let _ = s.sign_in(&store, "dana", "guess");
        }
        assert!(s.sign_in(&store, "dana", "correct horse battery").unwrap_err().contains("Too many"));
    }
}
