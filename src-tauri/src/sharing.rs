//! Links shared parts (skills, plugins, history...) of a source home into an account's home.
//! Directories: junction on Windows (no admin needed), symlink on unix.
//! Files: symlink, else hard link (Windows without Developer Mode), else a one-time copy.
//! Real user data is never deleted: anything in the way is moved to `<name>.hopper-backup-<stamp>`.
use crate::model::{Account, ShareKind, SharingOptions};
use crate::providers::ProviderAdapter;
use crate::util;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct SharingReport {
    pub linked: Vec<String>,
    pub unlinked: Vec<String>,
    /// Human-readable remarks: backups made, copies instead of links, errors per item.
    pub notes: Vec<String>,
}

/// Where each kind of shared item is linked from.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sources<'a> {
    /// Skills, plugins, agents, instructions, settings and MCP.
    pub knowledge: Option<&'a Path>,
    /// Chat history: stays where the old chats live so they can still be resumed.
    pub history: Option<&'a Path>,
    /// Before a real folder in the home is moved aside for a knowledge link, copy its files the
    /// knowledge source lacks into it (the source is a folder we own, not a tool home).
    pub adopt: bool,
}

impl<'a> Sources<'a> {
    /// Every kind from one home.
    pub fn same(home: Option<&'a Path>) -> Self {
        Self { knowledge: home, history: home, adopt: false }
    }

    pub fn for_kind(&self, kind: ShareKind) -> Option<&'a Path> {
        if kind == ShareKind::History {
            self.history
        } else {
            self.knowledge
        }
    }
}

/// Apply an account's sharing options using its provider's path lists. History links from
/// `shared_from`; everything else from `knowledge` when given, else from `shared_from` too.
/// The tool's default home is never changed.
pub fn apply_for_account(account: &Account, adapter: &dyn ProviderAdapter, knowledge: Option<&Path>) -> SharingReport {
    let home = Path::new(&account.home_dir);
    if account.is_default_home || util::path_key(home) == util::path_key(&adapter.default_home()) {
        return SharingReport::default();
    }
    let shareable = adapter.shareable_paths();
    let creds = adapter.credential_paths();
    let history = account.shared_from.as_deref().map(Path::new);
    let sources = Sources { knowledge: knowledge.or(history), history, adopt: knowledge.is_some() };
    apply(home, sources, &account.sharing, &shareable, &creds)
}

pub fn apply(
    home: &Path,
    sources: Sources,
    opts: &SharingOptions,
    shareable: &[(ShareKind, Vec<&'static str>)],
    credentials: &[&str],
) -> SharingReport {
    let mut report = SharingReport::default();
    // Sharing a home with itself would create self-referencing links.
    let usable = |s: Option<&Path>| s.filter(|s| util::path_key(s) != util::path_key(home)).map(Path::to_path_buf);
    let history = usable(sources.history);
    for (kind, rels) in shareable {
        let source = usable(sources.for_kind(*kind));
        let adopt = sources.adopt && *kind != ShareKind::History;
        // A kind moved off the history source may still have hard links to that source's files
        // in the home (Windows without symlinks); those are ours to replace or remove.
        let previous = history.as_ref().filter(|h| source.as_ref().is_some_and(|s| util::path_key(s) != util::path_key(h)));
        for rel in rels {
            if !is_safe_rel(rel) || touches_credentials(rel, credentials) {
                report.notes.push(format!("{rel}: skipped (protected path)"));
                continue;
            }
            let dst = home.join(rel);
            let old = previous.map(|p| p.join(rel));
            let result = match &source {
                Some(src_home) if opts.enabled(*kind) => {
                    drop_hard_link(old.as_deref(), &dst).and_then(|_| share_one(&src_home.join(rel), &dst, rel, adopt, &mut report))
                }
                _ => {
                    let ours: Vec<PathBuf> = source.iter().map(|s| s.join(rel)).chain(old).collect();
                    unshare_one(&ours, &dst, rel, &mut report)
                }
            };
            if let Err(e) = result {
                report.notes.push(format!("{rel}: {e}"));
            }
        }
    }
    report
}

/// Remove `dst` when it is a hard link to `old` (the data stays in `old`).
fn drop_hard_link(old: Option<&Path>, dst: &Path) -> io::Result<()> {
    if let Some(old) = old {
        let is_file = fs::symlink_metadata(dst).map(|m| m.is_file()).unwrap_or(false);
        if is_file && same_file::is_same_file(old, dst).unwrap_or(false) {
            fs::remove_file(dst)?;
        }
    }
    Ok(())
}

pub(crate) fn is_safe_rel(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// True when `rel` is a credential path, lies inside one, or contains one.
pub(crate) fn touches_credentials(rel: &str, credentials: &[&str]) -> bool {
    let r = util::path_key(Path::new(rel));
    credentials.iter().any(|c| {
        let c = util::path_key(Path::new(c));
        r == c || r.starts_with(&format!("{c}/")) || c.starts_with(&format!("{r}/"))
    })
}

/// Guess whether a missing shared item is a file: it has an extension (CLAUDE.md, settings.json).
fn looks_like_file(rel: &str) -> bool {
    Path::new(rel).extension().is_some()
}

/// `adopt`: before a real folder in the home is moved aside, copy its files that the source
/// lacks into the source, so nothing made in this account disappears from view.
fn share_one(src: &Path, dst: &Path, rel: &str, adopt: bool, report: &mut SharingReport) -> io::Result<()> {
    let is_dir = match fs::metadata(src) {
        Ok(m) => m.is_dir(),
        Err(_) if looks_like_file(rel) => return Ok(()), // nothing to share yet
        Err(_) => {
            fs::create_dir_all(src)?;
            true
        }
    };
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if is_link(dst) {
            if points_to(dst, src) {
                return Ok(());
            }
            remove_link(dst)?;
        } else if !is_dir && meta.is_file() && (same_file::is_same_file(src, dst).unwrap_or(false) || same_content(src, dst)) {
            return Ok(()); // our hard link or an unchanged copy
        } else if meta.is_dir() && is_empty_dir(dst) {
            fs::remove_dir(dst)?;
        } else {
            if adopt && is_dir && meta.is_dir() {
                let c = copy_missing(dst, src)?;
                if c.added > 0 {
                    report.notes.push(format!("{rel}: {} file(s) copied into {}", c.added, src.display()));
                }
            }
            let backup = move_aside(dst)?;
            report.notes.push(format!("{rel}: existing item moved to {}", backup.display()));
        }
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let how = if is_dir { link_dir(src, dst)? } else { link_file(src, dst)? };
    if how == LinkHow::Copy {
        report.notes.push(format!("{rel}: copied (links not possible here), changes will not sync"));
    }
    report.linked.push(rel.to_string());
    Ok(())
}

/// Remove only what we created: links (symlink/junction) and hard links to a source. Real data stays.
fn unshare_one(sources: &[PathBuf], dst: &Path, rel: &str, report: &mut SharingReport) -> io::Result<()> {
    let Ok(meta) = fs::symlink_metadata(dst) else {
        return Ok(());
    };
    if is_link(dst) {
        remove_link(dst)?;
        report.unlinked.push(rel.to_string());
    } else if meta.is_file() && sources.iter().any(|s| same_file::is_same_file(s, dst).unwrap_or(false)) {
        fs::remove_file(dst)?;
        report.unlinked.push(rel.to_string());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum LinkHow {
    Link,
    Copy,
}

pub fn is_link(p: &Path) -> bool {
    let sym = fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    #[cfg(windows)]
    {
        sym || junction::exists(p).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        sym
    }
}

fn points_to(link: &Path, target: &Path) -> bool {
    let read = fs::read_link(link).ok().or_else(|| {
        #[cfg(windows)]
        {
            junction::get_target(link).ok()
        }
        #[cfg(not(windows))]
        {
            None
        }
    });
    if let Some(t) = read {
        let t = if t.is_relative() { link.parent().map(|p| p.join(&t)).unwrap_or(t) } else { t };
        if norm_key(&t) == norm_key(target) {
            return true;
        }
    }
    matches!((fs::canonicalize(link), fs::canonicalize(target)), (Ok(a), Ok(b)) if a == b)
}

/// path_key without the Windows verbatim prefix that junction targets carry.
fn norm_key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").or_else(|| s.strip_prefix(r"\??\")).unwrap_or(&s);
    util::path_key(Path::new(s))
}

fn remove_link(p: &Path) -> io::Result<()> {
    // Unix: every symlink is a file entry. Windows: dir symlinks and junctions need remove_dir.
    fs::remove_file(p).or_else(|_| fs::remove_dir(p))
}

fn is_empty_dir(p: &Path) -> bool {
    fs::read_dir(p).map(|mut d| d.next().is_none()).unwrap_or(false)
}

fn same_content(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() && ma.len() < 16 * 1024 * 1024 => {
            matches!((fs::read(a), fs::read(b)), (Ok(x), Ok(y)) if x == y)
        }
        _ => false,
    }
}

/// Files handled by `copy_missing`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CopyCount {
    /// Copied because the target lacked them.
    pub added: usize,
    /// Already in the target, left as they were.
    pub kept: usize,
}

impl std::ops::AddAssign for CopyCount {
    fn add_assign(&mut self, o: Self) {
        self.added += o.added;
        self.kept += o.kept;
    }
}

/// Never copied: repo internals, installed packages and our own backups.
fn skip_on_copy(name: &str) -> bool {
    name == ".git" || name == "node_modules" || name.contains(".hopper-backup-")
}

/// Copy `src` (file or folder, links followed) to `dst`, adding only files `dst` lacks.
/// Never overwrites, moves or deletes anything.
pub fn copy_missing(src: &Path, dst: &Path) -> io::Result<CopyCount> {
    copy_missing_at(src, dst, 0)
}

fn copy_missing_at(src: &Path, dst: &Path, depth: usize) -> io::Result<CopyCount> {
    let mut count = CopyCount::default();
    // Links are followed, so a link loop would otherwise recurse forever.
    if depth > 32 {
        return Ok(count);
    }
    let meta = fs::metadata(src)?;
    if !meta.is_dir() {
        if fs::symlink_metadata(dst).is_ok() {
            count.kept += 1;
            return Ok(count);
        }
        let created = fs::OpenOptions::new().write(true).create_new(true).open(dst);
        match created {
            Ok(mut out) => {
                io::copy(&mut fs::File::open(src)?, &mut out)?;
                let _ = fs::set_permissions(dst, meta.permissions());
                count.added += 1;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => count.kept += 1,
            Err(e) => return Err(e),
        }
        return Ok(count);
    }
    if fs::symlink_metadata(dst).is_ok() && !dst.is_dir() {
        return Ok(count); // something else where a folder would go: leave it
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        // Skip broken links instead of failing the whole copy.
        if skip_on_copy(&name.to_string_lossy()) || fs::metadata(entry.path()).is_err() {
            continue;
        }
        count += copy_missing_at(&entry.path(), &dst.join(&name), depth + 1)?;
    }
    Ok(count)
}

fn move_aside(p: &Path) -> io::Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "item".into());
    let mut backup = p.with_file_name(format!("{name}.hopper-backup-{stamp}"));
    let mut n = 1;
    while backup.exists() {
        backup = p.with_file_name(format!("{name}.hopper-backup-{stamp}-{n}"));
        n += 1;
    }
    fs::rename(p, &backup)?;
    Ok(backup)
}

fn absolute(p: &Path) -> io::Result<PathBuf> {
    std::path::absolute(p)
}

#[cfg(windows)]
fn link_dir(src: &Path, dst: &Path) -> io::Result<LinkHow> {
    junction::create(absolute(src)?, dst)?;
    Ok(LinkHow::Link)
}

#[cfg(not(windows))]
fn link_dir(src: &Path, dst: &Path) -> io::Result<LinkHow> {
    std::os::unix::fs::symlink(absolute(src)?, dst)?;
    Ok(LinkHow::Link)
}

#[cfg(windows)]
fn link_file(src: &Path, dst: &Path) -> io::Result<LinkHow> {
    let src = absolute(src)?;
    if std::os::windows::fs::symlink_file(&src, dst).is_ok() || fs::hard_link(&src, dst).is_ok() {
        return Ok(LinkHow::Link);
    }
    fs::copy(&src, dst)?;
    Ok(LinkHow::Copy)
}

#[cfg(not(windows))]
fn link_file(src: &Path, dst: &Path) -> io::Result<LinkHow> {
    std::os::unix::fs::symlink(absolute(src)?, dst)?;
    Ok(LinkHow::Link)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<(ShareKind, Vec<&'static str>)> {
        vec![
            (ShareKind::Skills, vec!["skills"]),
            (ShareKind::Instructions, vec!["CLAUDE.md"]),
            (ShareKind::History, vec!["projects"]),
            (ShareKind::Settings, vec![".credentials.json"]),
        ]
    }

    const CREDS: &[&str] = &[".credentials.json"];

    fn all_on() -> SharingOptions {
        SharingOptions { skills: true, plugins: true, agents: true, instructions: true, settings: true, mcp: true, history: true }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("main home");
        let home = t.path().join("profile home");
        fs::create_dir_all(src.join("skills")).unwrap();
        fs::write(src.join("skills").join("a.txt"), "skill").unwrap();
        fs::write(src.join("CLAUDE.md"), "instructions").unwrap();
        fs::write(src.join(".credentials.json"), "secret").unwrap();
        fs::create_dir_all(&home).unwrap();
        (t, src, home)
    }

    #[test]
    fn links_dirs_and_files_and_creates_missing_source_dirs() {
        let (_t, src, home) = setup();
        let r = apply(&home, Sources::same(Some(&src)), &all_on(), &paths(), CREDS);
        assert_eq!(fs::read_to_string(home.join("skills").join("a.txt")).unwrap(), "skill");
        assert!(is_link(&home.join("skills")));
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), "instructions");
        assert!(src.join("projects").is_dir(), "missing source dir is created");
        assert!(is_link(&home.join("projects")));
        assert!(!home.join(".credentials.json").exists(), "credentials never linked");
        assert!(r.linked.contains(&"skills".to_string()));
    }

    #[test]
    fn apply_is_idempotent() {
        let (_t, src, home) = setup();
        apply(&home, Sources::same(Some(&src)), &all_on(), &paths(), CREDS);
        let r = apply(&home, Sources::same(Some(&src)), &all_on(), &paths(), CREDS);
        assert!(r.linked.is_empty(), "nothing relinked: {r:?}");
        let backups = fs::read_dir(&home).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains("hopper-backup")).count();
        assert_eq!(backups, 0);
    }

    #[test]
    fn existing_real_data_is_moved_aside_and_empty_dirs_replaced() {
        let (_t, src, home) = setup();
        fs::create_dir_all(home.join("skills")).unwrap();
        fs::write(home.join("skills").join("mine.txt"), "keep me").unwrap();
        fs::create_dir_all(home.join("projects")).unwrap();
        let r = apply(&home, Sources::same(Some(&src)), &all_on(), &paths(), CREDS);
        assert!(is_link(&home.join("skills")));
        let backup = fs::read_dir(&home)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("skills.hopper-backup-"))
            .expect("backup exists");
        assert_eq!(fs::read_to_string(backup.path().join("mine.txt")).unwrap(), "keep me");
        assert_eq!(r.notes.iter().filter(|n| n.contains("moved")).count(), 1, "{r:?}");
    }

    #[test]
    fn disabling_removes_only_links_and_keeps_source_data() {
        let (_t, src, home) = setup();
        apply(&home, Sources::same(Some(&src)), &all_on(), &paths(), CREDS);
        fs::write(home.join("private.txt"), "x").unwrap();
        let off = SharingOptions { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false };
        let r = apply(&home, Sources::same(Some(&src)), &off, &paths(), CREDS);
        assert!(!home.join("skills").exists());
        assert!(!home.join("CLAUDE.md").exists());
        assert!(home.join("private.txt").exists());
        assert_eq!(fs::read_to_string(src.join("skills").join("a.txt")).unwrap(), "skill");
        assert_eq!(fs::read_to_string(src.join("CLAUDE.md")).unwrap(), "instructions");
        assert!(r.unlinked.contains(&"skills".to_string()));
    }

    #[test]
    fn real_files_survive_disable() {
        let (_t, src, home) = setup();
        fs::write(home.join("CLAUDE.md"), "own").unwrap();
        let off = SharingOptions { instructions: false, ..all_on() };
        apply(&home, Sources::same(Some(&src)), &off, &paths(), CREDS);
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), "own");
    }

    #[test]
    fn self_sharing_and_unsafe_paths_are_ignored() {
        let (_t, src, _home) = setup();
        let r = apply(&src, Sources::same(Some(&src)), &all_on(), &paths(), CREDS);
        assert!(r.linked.is_empty());
        assert!(!is_safe_rel("../x"));
        assert!(touches_credentials("auth.json", &["auth.json"]));
        assert!(touches_credentials("dir", &["dir/token"]));
        assert!(!touches_credentials("skills", &["auth.json"]));
    }

    #[test]
    fn history_and_knowledge_link_from_their_own_sources() {
        let (t, main, home) = setup();
        let shared = t.path().join("shared");
        fs::create_dir_all(shared.join("skills")).unwrap();
        fs::write(shared.join("skills").join("b.txt"), "shared skill").unwrap();
        fs::write(shared.join("CLAUDE.md"), "shared instructions").unwrap();
        fs::create_dir_all(main.join("projects")).unwrap();
        fs::write(main.join("projects").join("chat.jsonl"), "old chat").unwrap();
        let sources = Sources { knowledge: Some(&shared), history: Some(&main), adopt: false };
        apply(&home, sources, &all_on(), &paths(), CREDS);
        assert!(is_link(&home.join("skills")));
        assert!(points_to(&home.join("skills"), &shared.join("skills")));
        assert!(!home.join("skills").join("a.txt").exists(), "main's skills are not linked");
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), "shared instructions");
        assert!(points_to(&home.join("projects"), &main.join("projects")));
        assert_eq!(fs::read_to_string(home.join("projects").join("chat.jsonl")).unwrap(), "old chat");
    }

    #[test]
    fn switching_the_knowledge_source_relinks_without_touching_either_source() {
        let (t, main, home) = setup();
        apply(&home, Sources::same(Some(&main)), &all_on(), &paths(), CREDS);
        assert!(points_to(&home.join("skills"), &main.join("skills")));
        let shared = t.path().join("shared");
        fs::create_dir_all(shared.join("skills")).unwrap();
        fs::write(shared.join("CLAUDE.md"), "shared").unwrap();
        let r = apply(&home, Sources { knowledge: Some(&shared), history: Some(&main), adopt: false }, &all_on(), &paths(), CREDS);
        assert!(points_to(&home.join("skills"), &shared.join("skills")));
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), "shared");
        assert!(points_to(&home.join("projects"), &main.join("projects")));
        assert_eq!(fs::read_to_string(main.join("skills").join("a.txt")).unwrap(), "skill");
        assert_eq!(fs::read_to_string(main.join("CLAUDE.md")).unwrap(), "instructions");
        assert!(!r.notes.iter().any(|n| n.contains("moved")), "{r:?}");
    }

    #[test]
    fn real_folder_is_merged_into_the_shared_folder_before_it_is_moved_aside() {
        let (t, main, home) = setup();
        let shared = t.path().join("shared");
        fs::create_dir_all(shared.join("skills").join("common")).unwrap();
        fs::write(shared.join("skills").join("common").join("SKILL.md"), "shared version").unwrap();
        fs::create_dir_all(home.join("skills").join("common")).unwrap();
        fs::write(home.join("skills").join("common").join("SKILL.md"), "account version").unwrap();
        fs::create_dir_all(home.join("skills").join("mine")).unwrap();
        fs::write(home.join("skills").join("mine").join("SKILL.md"), "made here").unwrap();
        let sources = Sources { knowledge: Some(&shared), history: Some(&main), adopt: true };
        let r = apply(&home, sources, &all_on(), &paths(), CREDS);
        assert!(points_to(&home.join("skills"), &shared.join("skills")));
        assert_eq!(fs::read_to_string(home.join("skills").join("mine").join("SKILL.md")).unwrap(), "made here");
        assert_eq!(fs::read_to_string(shared.join("skills").join("common").join("SKILL.md")).unwrap(), "shared version", "never overwritten");
        let backup = fs::read_dir(&home)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("skills.hopper-backup-"))
            .expect("backup kept");
        assert_eq!(fs::read_to_string(backup.path().join("common").join("SKILL.md")).unwrap(), "account version");
        assert!(r.notes.iter().any(|n| n.contains("1 file(s) copied")), "{r:?}");
    }

    #[test]
    fn copy_missing_adds_only_new_files_and_skips_repo_internals() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir_all(a.join("x").join(".git")).unwrap();
        fs::create_dir_all(a.join("node_modules")).unwrap();
        fs::write(a.join("x").join(".git").join("HEAD"), "ref").unwrap();
        fs::write(a.join("node_modules").join("dep.js"), "dep").unwrap();
        fs::write(a.join("x").join("new.txt"), "new").unwrap();
        fs::write(a.join("same.txt"), "from a").unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("same.txt"), "from b").unwrap();
        let c = copy_missing(&a, &b).unwrap();
        assert_eq!(c, CopyCount { added: 1, kept: 1 });
        assert_eq!(fs::read_to_string(b.join("same.txt")).unwrap(), "from b");
        assert_eq!(fs::read_to_string(b.join("x").join("new.txt")).unwrap(), "new");
        assert!(!b.join("x").join(".git").exists());
        assert!(!b.join("node_modules").exists());
        assert_eq!(fs::read_to_string(a.join("same.txt")).unwrap(), "from a", "source untouched");
    }
}
