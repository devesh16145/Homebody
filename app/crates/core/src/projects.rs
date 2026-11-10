//! Projects: letting a protected agent work on your files without ever touching them.
//!
//! "Bring a project in" makes a copy-on-write clone of a folder inside the agent's workspace (and
//! a second, private clone that only you can reach, the baseline). The agent works on its clone.
//! You review what changed and apply it back. Three properties fall out of this shape:
//!
//! - the agent never needs access to your home folder, so no layer of the fence is weakened;
//! - the agent cannot damage your project, only its copy;
//! - if you edited a file the agent also edited, that is noticed and nothing is overwritten.
//!
//! A project's own `.git` folder is neither compared nor copied back: changes arrive in your
//! folder as ordinary uncommitted edits for you to review and commit.

use crate::config::{AppConfig, ProjectConfig};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Folder names that are never compared or copied back, at any depth.
const SKIPPED: &[&str] = &[".git"];
/// `O_NOFOLLOW` on macOS and the BSDs.
const O_NOFOLLOW: i32 = 0x0100;
/// How many changes are listed for display. Counts are always complete.
pub const LIST_LIMIT: usize = 500;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    /// "added" | "modified" | "deleted"
    pub kind: String,
    /// You changed this too since the project was brought in (or last applied).
    pub conflict: bool,
    /// The agent's version is a link to another path, not a file. Shown so it can be judged.
    pub link: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub total: usize,
    pub conflicts: usize,
    pub items: Vec<Change>,
    pub truncated: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub applied: usize,
    pub skipped: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub name: String,
    pub source: String,
    pub copy: String,
    pub imported_at: String,
    pub source_exists: bool,
    pub copy_exists: bool,
}

pub fn copy_dir(app: &AppConfig, name: &str) -> Option<PathBuf> {
    app.workspace.as_ref().map(|w| Path::new(w).join(name))
}

/// The baseline lives in your own private data folder, out of the agent's reach.
pub fn base_dir(app_id: &str, name: &str) -> PathBuf {
    crate::paths::data_dir().join("projects").join(app_id).join(name)
}

pub fn info(app: &AppConfig, p: &ProjectConfig) -> ProjectInfo {
    let copy = copy_dir(app, &p.name).unwrap_or_default();
    ProjectInfo { name: p.name.clone(), source: p.source.clone(), copy_exists: copy.is_dir(), copy: copy.to_string_lossy().into_owned(), imported_at: p.imported_at.clone(), source_exists: Path::new(&p.source).is_dir() }
}

fn safe_name(raw: &str) -> String {
    let s: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '-' }).collect();
    let s = s.trim_matches(|c| c == '.' || c == '-').to_string();
    if s.is_empty() { "project".into() } else { s.chars().take(60).collect() }
}

/// Is this a folder it makes sense to bring in? Returns its real path.
pub fn validate_source(raw: &str, workspace: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(raw.trim()).map_err(|_| "That folder could not be found.".to_string())?;
    if !path.is_dir() {
        return Err("That is a file. Choose a folder.".into());
    }
    if let Ok(ws) = fs::canonicalize(workspace) {
        if path.starts_with(&ws) || ws.starts_with(&path) {
            return Err("That folder is already inside the protected workspace.".into());
        }
    }
    if let Some(home) = dirs::home_dir().and_then(|h| fs::canonicalize(h).ok()) {
        if path == home || path.parent() == Some(home.as_path()) || home.starts_with(&path) {
            return Err("Choose one project folder, not a whole area such as Documents or your home folder.".into());
        }
    }
    if path.components().count() < 3 {
        return Err("Choose one project folder, not a whole disk or system area.".into());
    }
    Ok(path)
}

fn clone_tree(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // `cp -c` clones on APFS (instant, no extra space until files diverge) and copies elsewhere.
    let out = std::process::Command::new("/bin/cp").arg("-cR").arg(from).arg(to).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("The folder could not be copied: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").trim()))
    }
}

/// Bring a folder in: one clone for the agent, one private baseline.
pub fn import(app: &AppConfig, source: &Path) -> Result<ProjectConfig, String> {
    let workspace = app.workspace.as_ref().map(PathBuf::from).ok_or("This app has no workspace.")?;
    if !workspace.is_dir() {
        return Err("The workspace does not exist yet. Finish setup first.".into());
    }
    let wanted = safe_name(&source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let taken = |n: &str| workspace.join(n).exists() || base_dir(&app.id, n).exists() || app.projects.iter().any(|p| p.name == n);
    let mut name = wanted.clone();
    let mut i = 2;
    while taken(&name) {
        name = format!("{wanted}-{i}");
        i += 1;
    }
    let (copy, base) = (workspace.join(&name), base_dir(&app.id, &name));
    clone_tree(source, &copy)?;
    if let Err(e) = clone_tree(source, &base) {
        let _ = fs::remove_dir_all(&copy);
        return Err(e);
    }
    Ok(ProjectConfig { name, source: source.to_string_lossy().into_owned(), imported_at: crate::now_iso() })
}

pub fn remove(app: &AppConfig, name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err("Unknown project.".into());
    }
    if let Some(copy) = copy_dir(app, name) {
        if copy.exists() {
            fs::remove_dir_all(&copy).map_err(|e| format!("The copy could not be removed: {e}"))?;
        }
    }
    let base = base_dir(&app.id, name);
    if base.exists() {
        let _ = fs::remove_dir_all(&base);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum Entry {
    File { len: u64, mtime: Option<SystemTime>, exec: bool },
    Link(PathBuf),
}

fn entry_at(path: &Path) -> Option<Entry> {
    let md = fs::symlink_metadata(path).ok()?;
    let ft = md.file_type();
    if ft.is_symlink() {
        Some(Entry::Link(fs::read_link(path).ok()?))
    } else if ft.is_file() {
        Some(Entry::File { len: md.len(), mtime: md.modified().ok(), exec: md.mode() & 0o111 != 0 })
    } else {
        None
    }
}

/// Every file and link under `root`, by relative path. Links are never followed.
fn scan(root: &Path) -> Result<BTreeMap<String, Entry>, String> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, Entry>) -> std::io::Result<()> {
        for item in fs::read_dir(dir)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let ft = item.file_type()?;
            if ft.is_dir() {
                if !SKIPPED.contains(&name.as_str()) {
                    walk(&item.path(), &child_rel, out)?;
                }
            } else if let Some(e) = entry_at(&item.path()) {
                out.insert(child_rel, e);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, "", &mut out).map_err(|e| format!("A folder could not be read: {e}"))?;
    Ok(out)
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let (Ok(na), Ok(nb)) = (fa.read(&mut ba), fb.read(&mut bb)) else { return false };
        if na != nb || ba[..na] != bb[..nb] {
            // Short reads can differ in length without the files differing; fall back to whole reads.
            return match (fs::read(a), fs::read(b)) {
                (Ok(x), Ok(y)) => x == y,
                _ => false,
            };
        }
        if na == 0 {
            return true;
        }
    }
}

/// Are the entries at `rel` under two roots the same thing?
fn same(a_root: &Path, a: &Entry, b_root: &Path, b: &Entry, rel: &str) -> bool {
    match (a, b) {
        (Entry::Link(x), Entry::Link(y)) => x == y,
        (Entry::File { len: la, mtime: ma, exec: ea }, Entry::File { len: lb, mtime: mb, exec: eb }) => ea == eb && la == lb && ((ma.is_some() && ma == mb) || same_bytes(&a_root.join(rel), &b_root.join(rel))),
        _ => false,
    }
}

/// What the agent changed in its copy, and whether you changed the same things meanwhile.
pub fn changes(copy: &Path, base: &Path, source: &Path) -> Result<Vec<Change>, String> {
    let (in_copy, in_base) = (scan(copy)?, scan(base)?);
    let mut out = Vec::new();
    let yours = |rel: &str| entry_at(&source.join(rel));
    for (rel, c) in &in_copy {
        let link = match c {
            Entry::Link(t) => Some(t.to_string_lossy().into_owned()),
            _ => None,
        };
        match in_base.get(rel) {
            None => {
                // New in the copy. A conflict if you also made something different at that path.
                let conflict = yours(rel).map(|s| !same(copy, c, source, &s, rel)).unwrap_or(false);
                let already = yours(rel).map(|s| same(copy, c, source, &s, rel)).unwrap_or(false);
                if !already {
                    out.push(Change { path: rel.clone(), kind: "added".into(), conflict, link });
                }
            }
            Some(b) if !same(copy, c, base, b, rel) => {
                let s = yours(rel);
                let you_changed = s.as_ref().map(|s| !same(base, b, source, s, rel)).unwrap_or(true);
                let identical = s.as_ref().map(|s| same(copy, c, source, s, rel)).unwrap_or(false);
                if !identical {
                    out.push(Change { path: rel.clone(), kind: "modified".into(), conflict: you_changed, link });
                }
            }
            Some(_) => {}
        }
    }
    for (rel, b) in &in_base {
        if !in_copy.contains_key(rel) {
            // Deleted in the copy. Nothing to do if it is already gone from yours.
            if let Some(s) = yours(rel) {
                out.push(Change { path: rel.clone(), kind: "deleted".into(), conflict: !same(base, b, source, &s, rel), link: None });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn change_set(copy: &Path, base: &Path, source: &Path) -> Result<ChangeSet, String> {
    let all = changes(copy, base, source)?;
    let conflicts = all.iter().filter(|c| c.conflict).count();
    let total = all.len();
    Ok(ChangeSet { total, conflicts, truncated: total > LIST_LIMIT, items: all.into_iter().take(LIST_LIMIT).collect() })
}

/// Copy one entry from one tree to another by content only: no ownership, no access lists, so
/// nothing about the protected user follows a file into your folder.
fn put(from_root: &Path, to_root: &Path, rel: &str) -> std::io::Result<()> {
    let (from, to) = (from_root.join(rel), to_root.join(rel));
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    match entry_at(&from) {
        Some(Entry::Link(target)) => {
            let _ = fs::remove_file(&to);
            std::os::unix::fs::symlink(target, &to)
        }
        Some(Entry::File { exec, .. }) => {
            if fs::symlink_metadata(&to).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                fs::remove_file(&to)?;
            }
            // The copy belongs to the agent and may change under us: refuse to read through a link.
            let mut src = { use std::os::unix::fs::OpenOptionsExt; fs::OpenOptions::new().read(true).custom_flags(O_NOFOLLOW).open(&from)? };
            let mut dst = fs::File::create(&to)?;
            std::io::copy(&mut src, &mut dst)?;
            fs::set_permissions(&to, fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }))
        }
        None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "nothing to copy")),
    }
}

fn take_away(root: &Path, rel: &str) {
    let _ = fs::remove_file(root.join(rel));
}

/// Apply the agent's changes to your folder. Conflicts are left alone unless `overwrite`.
/// The baseline is moved forward for everything applied, so the next review starts from here.
pub fn apply(copy: &Path, base: &Path, source: &Path, overwrite: bool) -> Result<ApplyResult, String> {
    if !source.is_dir() {
        return Err("Your original folder is no longer there, so the changes have nowhere to go.".into());
    }
    let mut result = ApplyResult { applied: 0, skipped: Vec::new() };
    for ch in changes(copy, base, source)? {
        if ch.path.split('/').any(|part| part == ".." || part.is_empty()) {
            continue;
        }
        if ch.conflict && !overwrite {
            result.skipped.push(ch.path);
            continue;
        }
        let done = if ch.kind == "deleted" {
            take_away(source, &ch.path);
            take_away(base, &ch.path);
            Ok(())
        } else {
            put(copy, source, &ch.path).and_then(|_| put(copy, base, &ch.path))
        };
        match done {
            Ok(()) => result.applied += 1,
            Err(e) => return Err(format!("Stopped at {}: {e}. {} changes were applied before that.", ch.path, result.applied)),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct World {
        _dir: tempfile::TempDir,
        source: PathBuf,
        copy: PathBuf,
        base: PathBuf,
    }

    /// A source with a few files, brought in the way `import` does it.
    fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mine/proj");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(source.join(".git")).unwrap();
        fs::write(source.join("README.md"), "hello\n").unwrap();
        fs::write(source.join("src/main.py"), "print('v1')\n").unwrap();
        fs::write(source.join("src/old.py"), "old\n").unwrap();
        fs::write(source.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let (copy, base) = (dir.path().join("workspace/proj"), dir.path().join("private/proj"));
        clone_tree(&source, &copy).unwrap();
        clone_tree(&source, &base).unwrap();
        World { _dir: dir, source, copy, base }
    }

    fn kinds(w: &World) -> Vec<(String, String, bool)> {
        changes(&w.copy, &w.base, &w.source).unwrap().into_iter().map(|c| (c.path, c.kind, c.conflict)).collect()
    }

    #[test]
    fn a_fresh_project_has_no_changes() {
        assert!(kinds(&world()).is_empty());
    }

    #[test]
    fn sees_added_modified_and_deleted_files() {
        let w = world();
        fs::write(w.copy.join("src/main.py"), "print('v2 from agent')\n").unwrap();
        fs::write(w.copy.join("src/new.py"), "new\n").unwrap();
        fs::remove_file(w.copy.join("src/old.py")).unwrap();
        assert_eq!(kinds(&w), vec![("src/main.py".into(), "modified".into(), false), ("src/new.py".into(), "added".into(), false), ("src/old.py".into(), "deleted".into(), false)]);
    }

    #[test]
    fn apply_copies_changes_back_and_then_nothing_is_pending() {
        let w = world();
        fs::write(w.copy.join("src/main.py"), "print('v2 from agent')\n").unwrap();
        fs::create_dir_all(w.copy.join("docs/deep")).unwrap();
        fs::write(w.copy.join("docs/deep/note.md"), "note\n").unwrap();
        fs::remove_file(w.copy.join("src/old.py")).unwrap();
        let r = apply(&w.copy, &w.base, &w.source, false).unwrap();
        assert_eq!((r.applied, r.skipped.len()), (3, 0));
        assert_eq!(fs::read_to_string(w.source.join("src/main.py")).unwrap(), "print('v2 from agent')\n");
        assert_eq!(fs::read_to_string(w.source.join("docs/deep/note.md")).unwrap(), "note\n");
        assert!(!w.source.join("src/old.py").exists());
        assert!(kinds(&w).is_empty(), "after applying, nothing is waiting");
    }

    #[test]
    fn your_own_edit_to_the_same_file_is_a_conflict_and_is_not_overwritten() {
        let w = world();
        fs::write(w.copy.join("src/main.py"), "print('agent')\n").unwrap();
        fs::write(w.source.join("src/main.py"), "print('you, meanwhile')\n").unwrap();
        fs::write(w.copy.join("README.md"), "hello from agent\n").unwrap();
        assert!(kinds(&w).contains(&("src/main.py".into(), "modified".into(), true)));
        let r = apply(&w.copy, &w.base, &w.source, false).unwrap();
        assert_eq!(r.applied, 1);
        assert_eq!(r.skipped, vec!["src/main.py".to_string()]);
        assert_eq!(fs::read_to_string(w.source.join("src/main.py")).unwrap(), "print('you, meanwhile')\n", "your edit must survive");
        assert_eq!(fs::read_to_string(w.source.join("README.md")).unwrap(), "hello from agent\n");
        // Asking for it explicitly takes the agent's version.
        let r = apply(&w.copy, &w.base, &w.source, true).unwrap();
        assert_eq!(r.applied, 1);
        assert_eq!(fs::read_to_string(w.source.join("src/main.py")).unwrap(), "print('agent')\n");
    }

    #[test]
    fn deleting_a_file_you_changed_is_a_conflict() {
        let w = world();
        fs::remove_file(w.copy.join("src/old.py")).unwrap();
        fs::write(w.source.join("src/old.py"), "you still need this\n").unwrap();
        assert_eq!(kinds(&w), vec![("src/old.py".into(), "deleted".into(), true)]);
        apply(&w.copy, &w.base, &w.source, false).unwrap();
        assert!(w.source.join("src/old.py").exists());
    }

    #[test]
    fn your_unrelated_edits_are_left_alone() {
        let w = world();
        fs::write(w.source.join("README.md"), "you edited the readme\n").unwrap();
        fs::write(w.copy.join("src/main.py"), "print('agent')\n").unwrap();
        assert_eq!(kinds(&w), vec![("src/main.py".into(), "modified".into(), false)]);
        apply(&w.copy, &w.base, &w.source, false).unwrap();
        assert_eq!(fs::read_to_string(w.source.join("README.md")).unwrap(), "you edited the readme\n");
    }

    #[test]
    fn git_internals_are_never_compared_or_copied_back() {
        let w = world();
        fs::write(w.copy.join(".git/HEAD"), "ref: refs/heads/agent-branch\n").unwrap();
        fs::create_dir_all(w.copy.join(".git/hooks")).unwrap();
        fs::write(w.copy.join(".git/hooks/pre-commit"), "#!/bin/sh\ncurl evil\n").unwrap();
        assert!(kinds(&w).is_empty());
        apply(&w.copy, &w.base, &w.source, true).unwrap();
        assert_eq!(fs::read_to_string(w.source.join(".git/HEAD")).unwrap(), "ref: refs/heads/main\n");
        assert!(!w.source.join(".git/hooks/pre-commit").exists(), "a hook planted in the copy must not reach your repository");
    }

    #[test]
    fn links_are_reported_as_links_and_never_followed() {
        let w = world();
        std::os::unix::fs::symlink("/etc/hosts", w.copy.join("hosts-link")).unwrap();
        let all = changes(&w.copy, &w.base, &w.source).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].link.as_deref(), Some("/etc/hosts"));
        apply(&w.copy, &w.base, &w.source, false).unwrap();
        let md = fs::symlink_metadata(w.source.join("hosts-link")).unwrap();
        assert!(md.file_type().is_symlink(), "applied as a link, not as a copy of what it points to");
    }

    #[test]
    fn a_directory_swapped_for_a_link_cannot_redirect_writes() {
        // The agent replaces src/ with a link to somewhere else and puts a file "inside" it.
        let w = world();
        let elsewhere = w._dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::remove_dir_all(w.copy.join("src")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, w.copy.join("src")).unwrap();
        fs::write(elsewhere.join("planted.py"), "x\n").unwrap();
        let all = changes(&w.copy, &w.base, &w.source).unwrap();
        // The link itself shows up as one change; nothing "inside" it is walked.
        assert!(all.iter().any(|c| c.path == "src" && c.link.is_some()));
        assert!(!all.iter().any(|c| c.path.contains("planted")));
    }

    #[test]
    fn executable_bit_changes_are_noticed() {
        let w = world();
        fs::set_permissions(w.copy.join("src/main.py"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(kinds(&w), vec![("src/main.py".into(), "modified".into(), false)]);
    }

    #[test]
    fn refuses_whole_areas_and_the_workspace_itself() {
        let ws = tempfile::tempdir().unwrap();
        assert!(validate_source("/", ws.path()).is_err());
        assert!(validate_source("/Users", ws.path()).is_err());
        if let Some(home) = dirs::home_dir() {
            assert!(validate_source(&home.to_string_lossy(), ws.path()).is_err());
            assert!(validate_source(&home.join("Documents").to_string_lossy(), ws.path()).is_err());
        }
        assert!(validate_source(&ws.path().to_string_lossy(), ws.path()).is_err());
        assert!(validate_source("/no/such/folder/anywhere", ws.path()).is_err());
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("My Project (v2)"), "My-Project--v2");
        assert_eq!(safe_name("..hidden"), "hidden");
        assert_eq!(safe_name("///"), "project");
    }
}
