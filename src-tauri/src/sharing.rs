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

/// Apply an account's sharing options using its provider's path lists.
pub fn apply_for_account(account: &Account, adapter: &dyn ProviderAdapter) -> SharingReport {
    let shareable = adapter.shareable_paths();
    let creds = adapter.credential_paths();
    apply(
        Path::new(&account.home_dir),
        account.shared_from.as_deref().map(Path::new),
        &account.sharing,
        &shareable,
        &creds,
    )
}

pub fn apply(
    home: &Path,
    shared_from: Option<&Path>,
    opts: &SharingOptions,
    shareable: &[(ShareKind, Vec<&'static str>)],
    credentials: &[&str],
) -> SharingReport {
    let mut report = SharingReport::default();
    // Sharing a home with itself would create self-referencing links.
    let source = shared_from.filter(|s| util::path_key(s) != util::path_key(home));
    for (kind, rels) in shareable {
        for rel in rels {
            if !is_safe_rel(rel) || touches_credentials(rel, credentials) {
                report.notes.push(format!("{rel}: skipped (protected path)"));
                continue;
            }
            let dst = home.join(rel);
            let result = match source {
                Some(src_home) if opts.enabled(*kind) => share_one(&src_home.join(rel), &dst, rel, &mut report),
                _ => unshare_one(source.map(|s| s.join(rel)).as_deref(), &dst, rel, &mut report),
            };
            if let Err(e) = result {
                report.notes.push(format!("{rel}: {e}"));
            }
        }
    }
    report
}

fn is_safe_rel(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// True when `rel` is a credential path, lies inside one, or contains one.
fn touches_credentials(rel: &str, credentials: &[&str]) -> bool {
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

fn share_one(src: &Path, dst: &Path, rel: &str, report: &mut SharingReport) -> io::Result<()> {
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

/// Remove only what we created: links (symlink/junction) and hard links to the source. Real data stays.
fn unshare_one(src: Option<&Path>, dst: &Path, rel: &str, report: &mut SharingReport) -> io::Result<()> {
    let Ok(meta) = fs::symlink_metadata(dst) else {
        return Ok(());
    };
    if is_link(dst) {
        remove_link(dst)?;
        report.unlinked.push(rel.to_string());
    } else if meta.is_file() && src.is_some_and(|s| same_file::is_same_file(s, dst).unwrap_or(false)) {
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
        let r = apply(&home, Some(&src), &all_on(), &paths(), CREDS);
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
        apply(&home, Some(&src), &all_on(), &paths(), CREDS);
        let r = apply(&home, Some(&src), &all_on(), &paths(), CREDS);
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
        let r = apply(&home, Some(&src), &all_on(), &paths(), CREDS);
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
        apply(&home, Some(&src), &all_on(), &paths(), CREDS);
        fs::write(home.join("private.txt"), "x").unwrap();
        let off = SharingOptions { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false };
        let r = apply(&home, Some(&src), &off, &paths(), CREDS);
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
        apply(&home, Some(&src), &off, &paths(), CREDS);
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), "own");
    }

    #[test]
    fn self_sharing_and_unsafe_paths_are_ignored() {
        let (_t, src, _home) = setup();
        let r = apply(&src, Some(&src), &all_on(), &paths(), CREDS);
        assert!(r.linked.is_empty());
        assert!(!is_safe_rel("../x"));
        assert!(touches_credentials("auth.json", &["auth.json"]));
        assert!(touches_credentials("dir", &["dir/token"]));
        assert!(!touches_credentials("skills", &["auth.json"]));
    }
}
