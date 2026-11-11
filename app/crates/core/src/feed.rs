//! The Feed's view of history: ledger records folded into one row per thing the user cares about.
//!
//! The same `apply` runs when replaying the ledger at startup and when a record is written live,
//! so what the user sees after a restart is exactly what they saw before it.

use crate::model::{Reason, Verb};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Row {
    pub id: u64,
    pub ts: String,
    pub app: String,
    pub host: String,
    pub port: u16,
    pub verb: Verb,
    pub reason: Reason,
    pub open: bool,
    pub bytes_out: Option<u64>,
    pub bytes_in: Option<u64>,
    pub duration_ms: Option<u64>,
    /// The connection record folded into this row, if any.
    pub conn: Option<u64>,
    /// Identical attempts that were not given rows of their own.
    pub repeats: u64,
    /// Allowed, but the destination could not be reached.
    pub unreachable: bool,
    /// This row began as a prompt to the user.
    pub asked: bool,
    /// Why exactly, when the reason alone is too coarse: "port", "greeting", "no_name", "not_encrypted".
    pub detail: String,
    /// For "greeting": the name the app tried to greet the server with.
    pub other: String,
    /// When the connection was looked inside: what was asked for, and how often.
    pub requests: Option<Vec<(String, u64)>>,
}

#[derive(Default)]
pub struct FeedIndex {
    rows: Vec<Row>,
    by_id: HashMap<u64, usize>,
    by_conn: HashMap<u64, usize>,
}

const MAX_ROWS: usize = 25_000;
const TRIM_TO: usize = 20_000;

impl FeedIndex {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn get(&self, id: u64) -> Option<&Row> {
        self.by_id.get(&id).map(|i| &self.rows[*i])
    }

    /// Fold one ledger record in. Returns the id of the row that was created or changed.
    pub fn apply(&mut self, rec: &Value) -> Option<u64> {
        let seq = rec.get("seq")?.as_u64()?;
        let s = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str| rec.get(k).and_then(Value::as_u64);
        match rec.get("kind")?.as_str()? {
            "ask" => {
                self.push(Row { id: seq, ts: s("ts"), app: s("app"), host: s("host"), port: n("port").unwrap_or(443) as u16, verb: Verb::Asked, reason: Reason::NotTrusted, open: true, bytes_out: None, bytes_in: None, duration_ms: None, conn: None, repeats: 0, unreachable: false, asked: true, detail: String::new(), other: String::new(), requests: None });
                Some(seq)
            }
            "answer" => {
                let idx = *self.by_id.get(&n("ask")?)?;
                let row = &mut self.rows[idx];
                let (verb, reason) = match rec.get("choice").and_then(Value::as_str)? {
                    "once" => (Verb::Allowed, Reason::YouAllowedOnce),
                    "always" => (Verb::Allowed, Reason::YouAlwaysAllow),
                    "block" => (Verb::Blocked, Reason::YouBlocked),
                    _ => (Verb::Blocked, Reason::NoAnswer),
                };
                row.verb = verb;
                row.reason = reason;
                row.open = false;
                Some(row.id)
            }
            "conn" => {
                let allow = rec.get("decision").and_then(Value::as_str) == Some("allow");
                let reason = Reason::parse(&s("reason")).unwrap_or(Reason::NotTrusted);
                // The first connection that waited on a prompt is shown on the prompt's own row.
                if allow {
                    if let Some(idx) = n("ask").and_then(|a| self.by_id.get(&a).copied()) {
                        if self.rows[idx].conn.is_none() {
                            self.rows[idx].conn = Some(seq);
                            self.rows[idx].open = true;
                            self.by_conn.insert(seq, idx);
                            return Some(self.rows[idx].id);
                        }
                    }
                }
                let verb = match (allow, reason) {
                    (true, Reason::Watching) | (true, Reason::ProtectionOff) => Verb::Seen,
                    (true, _) => Verb::Allowed,
                    (false, _) => Verb::Blocked,
                };
                self.push(Row { id: seq, ts: s("ts"), app: s("app"), host: s("host"), port: n("port").unwrap_or(443) as u16, verb, reason, open: allow, bytes_out: None, bytes_in: None, duration_ms: None, conn: if allow { Some(seq) } else { None }, repeats: n("repeats").unwrap_or(0), unreachable: false, asked: false, detail: s("detail"), other: s("other"), requests: None });
                if allow {
                    self.by_conn.insert(seq, self.rows.len() - 1);
                }
                Some(seq)
            }
            // Repeats counted during an earlier run. While running they are counted directly
            // (`add_repeat`) and this record is written later, so it is only folded in on replay.
            "repeat" => {
                let idx = *self.by_id.get(&n("of")?)?;
                self.rows[idx].repeats += n("count").unwrap_or(0);
                Some(self.rows[idx].id)
            }
            "end" => {
                let idx = self.by_conn.remove(&n("conn")?)?;
                let row = &mut self.rows[idx];
                row.bytes_out = Some(row.bytes_out.unwrap_or(0) + n("out").unwrap_or(0));
                row.bytes_in = Some(row.bytes_in.unwrap_or(0) + n("in").unwrap_or(0));
                row.duration_ms = n("ms");
                row.unreachable = rec.get("unreachable").and_then(Value::as_bool).unwrap_or(false);
                if let Some(paths) = rec.get("paths").and_then(Value::as_array) {
                    row.requests = Some(paths.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_u64()?))).collect());
                }
                row.open = false;
                Some(row.id)
            }
            _ => None,
        }
    }

    /// Count an identical attempt against an existing row instead of adding a new one.
    pub fn add_repeat(&mut self, id: u64) -> bool {
        match self.by_id.get(&id) {
            Some(i) => {
                self.rows[*i].repeats += 1;
                true
            }
            None => false,
        }
    }

    /// After a restart nothing from the previous run is still in progress.
    pub fn close_stale(&mut self) {
        for r in &mut self.rows {
            if r.open {
                r.open = false;
                if r.verb == Verb::Asked {
                    r.verb = Verb::Blocked;
                    r.reason = Reason::NoAnswer;
                }
            }
        }
        self.by_conn.clear();
    }

    fn push(&mut self, row: Row) {
        self.by_id.insert(row.id, self.rows.len());
        self.rows.push(row);
        if self.rows.len() > MAX_ROWS {
            let drop = self.rows.len() - TRIM_TO;
            self.rows.drain(..drop);
            self.by_id = self.rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
            self.by_conn = self.rows.iter().enumerate().filter(|(_, r)| r.open).filter_map(|(i, r)| r.conn.map(|c| (c, i))).collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_prompt_and_its_connection_share_one_row() {
        let mut f = FeedIndex::default();
        assert_eq!(f.apply(&json!({"seq":1,"kind":"ask","ts":"t","app":"ollama","host":"registry.ollama.ai","port":443})), Some(1));
        assert_eq!(f.get(1).unwrap().verb, Verb::Asked);
        assert_eq!(f.apply(&json!({"seq":2,"kind":"answer","ask":1,"choice":"once"})), Some(1));
        assert_eq!(f.get(1).unwrap().verb, Verb::Allowed);
        assert_eq!(f.apply(&json!({"seq":3,"kind":"conn","ts":"t","app":"ollama","host":"registry.ollama.ai","port":443,"decision":"allow","reason":"you_allowed_once","ask":1})), Some(1));
        assert!(f.get(1).unwrap().open);
        // A second connection under the same answer gets its own row.
        assert_eq!(f.apply(&json!({"seq":4,"kind":"conn","ts":"t","app":"ollama","host":"registry.ollama.ai","port":443,"decision":"allow","reason":"you_allowed_once","ask":1})), Some(4));
        assert_eq!(f.apply(&json!({"seq":5,"kind":"end","conn":3,"out":120,"in":4000,"ms":800})), Some(1));
        let r = f.get(1).unwrap();
        assert!(!r.open);
        assert_eq!((r.bytes_out, r.bytes_in), (Some(120), Some(4000)));
        assert_eq!(f.rows().len(), 2);
    }

    #[test]
    fn blocked_and_seen_rows() {
        let mut f = FeedIndex::default();
        f.apply(&json!({"seq":1,"kind":"conn","ts":"t","app":"a","host":"x.example.com","port":443,"decision":"deny","reason":"not_trusted"}));
        f.apply(&json!({"seq":2,"kind":"conn","ts":"t","app":"a","host":"y.example.com","port":443,"decision":"allow","reason":"watching"}));
        assert_eq!(f.get(1).unwrap().verb, Verb::Blocked);
        assert!(!f.get(1).unwrap().open);
        assert_eq!(f.get(2).unwrap().verb, Verb::Seen);
        assert!(f.get(2).unwrap().open);
    }

    #[test]
    fn repeats_survive_a_restart() {
        let mut f = FeedIndex::default();
        f.apply(&json!({"seq":1,"kind":"conn","ts":"t","app":"a","host":"x.example.com","port":443,"decision":"deny","reason":"not_trusted"}));
        f.apply(&json!({"seq":2,"kind":"repeat","of":1,"count":7}));
        f.apply(&json!({"seq":3,"kind":"repeat","of":1,"count":2}));
        assert_eq!(f.get(1).unwrap().repeats, 9);
        assert_eq!(f.apply(&json!({"seq":4,"kind":"repeat","of":99,"count":2})), None);
    }

    #[test]
    fn a_timeout_blocks() {
        let mut f = FeedIndex::default();
        f.apply(&json!({"seq":1,"kind":"ask","ts":"t","app":"a","host":"x.example.com","port":443}));
        f.apply(&json!({"seq":2,"kind":"answer","ask":1,"choice":"timeout"}));
        let r = f.get(1).unwrap();
        assert_eq!((r.verb, r.reason, r.open), (Verb::Blocked, Reason::NoAnswer, false));
    }

    #[test]
    fn stale_rows_are_closed_after_restart() {
        let mut f = FeedIndex::default();
        f.apply(&json!({"seq":1,"kind":"ask","ts":"t","app":"a","host":"x.example.com","port":443}));
        f.apply(&json!({"seq":2,"kind":"conn","ts":"t","app":"a","host":"y.example.com","port":443,"decision":"allow","reason":"trusted"}));
        f.close_stale();
        assert_eq!(f.get(1).unwrap().verb, Verb::Blocked);
        assert!(!f.get(2).unwrap().open);
    }
}
