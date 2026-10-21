//! Skills from the public skills.sh directory, installed into each tool's shared folder so every
//! account that links its skills gets them at once. The files come from the skill's GitHub
//! repository at one commit. Nothing is ever overwritten, and only folders Switchyard installed
//! (they carry a marker file) are ever removed.
use crate::model::ShareKind;
use crate::providers::ProviderAdapter;
use crate::{sharing, util};
use anyhow::{anyhow, bail, Context};
use std::fs;
use std::path::{Path, PathBuf};

/// Written into every skill folder Switchyard installs: where it came from.
pub const MARKER: &str = ".switchyard-skill.json";

/// Most files and bytes one skill may bring.
const MAX_FILES: usize = 500;
const MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Never installed: repo internals and installed packages.
const SKIPPED: [&str; 3] = [".git", "node_modules", "__pycache__"];

/// A skill that can be installed: "owner/repo/skill" on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRef {
    pub owner: String,
    pub repo: String,
    /// The directory's slug for the skill, also its folder name once installed.
    pub skill: String,
}

impl SkillRef {
    /// GitHub skills only: owners are letters, digits and dashes, so a domain (a well-known
    /// source) never parses. The skill must be a slug, which keeps it a plain folder name.
    pub fn parse(id: &str) -> Option<Self> {
        let parts: Vec<&str> = id.split('/').collect();
        let [owner, repo, skill] = parts.as_slice() else { return None };
        let owner_ok = !owner.is_empty() && owner.len() <= 39 && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let repo_ok = !repo.is_empty()
            && repo.len() <= 100
            && !matches!(*repo, "." | "..")
            && repo.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        let skill_ok = !skill.is_empty() && skill.len() <= 100 && skill_slug(skill) == *skill;
        (owner_ok && repo_ok && skill_ok).then(|| Self { owner: owner.to_string(), repo: repo.to_string(), skill: skill.to_string() })
    }

    pub fn id(&self) -> String {
        format!("{}/{}/{}", self.owner, self.repo, self.skill)
    }

    /// The GitHub repository, "owner/repo".
    pub fn source(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// skills.sh's slug for a skill name, as toSkillSlug() in the skills CLI makes it: lowercase,
/// spaces and underscores become dashes, anything else that is not a letter or digit is dropped.
pub fn skill_slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let c = if c.is_whitespace() || c == '_' { '-' } else { c };
        let keep = c.is_ascii_lowercase() || c.is_ascii_digit() || (c == '-' && !out.is_empty() && !out.ends_with('-'));
        if keep {
            out.push(c);
        }
    }
    out.trim_end_matches('-').to_string()
}

/// One file of a GitHub tree listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    pub size: u64,
    pub executable: bool,
    /// A symbolic link in the repository: never installed.
    pub link: bool,
}

/// Where `skill` lives in a repository: the folder whose SKILL.md sits in a folder named after it
/// (the skill spec asks for that). Folders under skills/ win, then the shallowest. A repository
/// that is itself one skill has its SKILL.md at the top, given as "".
pub fn pick_folder(files: &[TreeFile], skill: &str) -> Option<String> {
    let skill_mds = || files.iter().filter(|f| !f.link && is_skill_md(&f.path));
    let mut found: Vec<&str> = skill_mds()
        .filter_map(|f| f.path.rsplit_once('/').map(|(dir, _)| dir))
        .filter(|dir| dir.rsplit('/').next().is_some_and(|name| skill_slug(name) == skill))
        .collect();
    found.sort_by_key(|d| (!d.starts_with("skills/"), d.matches('/').count(), *d));
    match found.first() {
        Some(dir) => Some(dir.to_string()),
        None => skill_mds().any(|f| !f.path.contains('/')).then(String::new),
    }
}

fn is_skill_md(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
}

/// The files of the skill folder to download, relative to it, without links, repo internals and
/// installed packages. A repository that is one skill brings only its SKILL.md: the rest of the
/// repository is not part of the skill.
pub fn plan_files(files: &[TreeFile], folder: &str) -> anyhow::Result<Vec<TreeFile>> {
    let mut out = Vec::new();
    for f in files.iter().filter(|f| !f.link) {
        let rel = if folder.is_empty() {
            if !f.path.eq_ignore_ascii_case("SKILL.md") {
                continue;
            }
            f.path.as_str()
        } else {
            match f.path.strip_prefix(folder).and_then(|r| r.strip_prefix('/')) {
                Some(r) => r,
                None => continue,
            }
        };
        if rel.split('/').any(|part| SKIPPED.contains(&part)) {
            continue;
        }
        if !sharing::is_safe_rel(rel) || rel.contains('\\') {
            bail!("The skill has a file Switchyard can't save safely: {rel}");
        }
        out.push(TreeFile { path: rel.to_string(), ..f.clone() });
    }
    if !out.iter().any(|f| f.path.eq_ignore_ascii_case("SKILL.md")) {
        bail!("The skill's folder has no SKILL.md");
    }
    let bytes: u64 = out.iter().map(|f| f.size).sum();
    if out.len() > MAX_FILES || bytes > MAX_BYTES {
        bail!("This skill is too big to install here ({} files, {} MB)", out.len(), bytes.div_ceil(1024 * 1024));
    }
    Ok(out)
}

/// A skill's files, downloaded and ready to write. Paths are relative to the skill folder.
#[derive(Debug, Clone)]
pub struct Download {
    pub commit: String,
    /// The skill's folder in the repository ("" for a repository that is one skill).
    pub folder: String,
    pub files: Vec<(TreeFile, Vec<u8>)>,
}

/// Write the skill to `<skills_dir>/<skill>` through a temporary folder beside the skills folder,
/// so a failed install never leaves half a skill where the tools look. Fails when anything is
/// already at that name.
pub fn write_skill(skills_dir: &Path, r: &SkillRef, d: &Download) -> anyhow::Result<PathBuf> {
    let target = skills_dir.join(&r.skill);
    if fs::symlink_metadata(&target).is_ok() {
        bail!("{} is already there", target.display());
    }
    fs::create_dir_all(skills_dir).with_context(|| format!("creating {}", skills_dir.display()))?;
    let tmp = skills_dir
        .parent()
        .unwrap_or(skills_dir)
        .join(format!(".switchyard-install-{}", uuid::Uuid::new_v4().simple()));
    let written = fill(&tmp, r, d).and_then(|_| fs::rename(&tmp, &target).map_err(Into::into));
    if written.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    written.with_context(|| format!("writing {}", target.display()))?;
    Ok(target)
}

fn fill(dir: &Path, r: &SkillRef, d: &Download) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    for (f, bytes) in &d.files {
        let path = dir.join(&f.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)?;
        #[cfg(unix)]
        if f.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    let marker = serde_json::json!({ "id": r.id(), "commit": d.commit, "folder": d.folder, "installedAt": util::now_iso() });
    fs::write(dir.join(MARKER), serde_json::to_vec_pretty(&marker)?)?;
    Ok(())
}

/// The directory id of a skill folder Switchyard installed, from its marker.
pub fn installed_id(dir: &Path) -> Option<String> {
    util::read_json(&dir.join(MARKER))?.get("id")?.as_str().map(str::to_string)
}

/// Delete `<skills_dir>/<skill>` when Switchyard installed it from `r`. A link, or a folder that
/// someone else put there, is left alone.
pub fn remove_skill(skills_dir: &Path, r: &SkillRef) -> anyhow::Result<()> {
    let target = skills_dir.join(&r.skill);
    let meta = fs::symlink_metadata(&target).map_err(|_| anyhow!("It isn't installed"))?;
    if !meta.is_dir() || sharing::is_link(&target) || installed_id(&target).as_deref() != Some(r.id().as_str()) {
        bail!("Switchyard didn't install this copy, so it was left alone");
    }
    fs::remove_dir_all(&target).with_context(|| format!("deleting {}", target.display()))
}

/// The folder a tool reads skills from inside its shared folder: its first skills path.
pub fn skills_dir(shared: &Path, adapter: &dyn ProviderAdapter) -> Option<PathBuf> {
    let (_, rels) = adapter.shareable_paths().into_iter().find(|(k, _)| *k == ShareKind::Skills)?;
    rels.first().map(|rel| shared.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> TreeFile {
        TreeFile { path: path.into(), size, executable: false, link: false }
    }

    fn pdf() -> SkillRef {
        SkillRef::parse("anthropics/skills/pdf").unwrap()
    }

    fn download(files: &[(&str, &str)]) -> Download {
        Download {
            commit: "a".repeat(40),
            folder: "skills/pdf".into(),
            files: files.iter().map(|(p, c)| (file(p, c.len() as u64), c.as_bytes().to_vec())).collect(),
        }
    }

    #[test]
    fn parses_github_ids_only() {
        let r = SkillRef::parse("vercel-labs/agent-skills/vercel-react-best-practices").unwrap();
        assert_eq!((r.owner.as_str(), r.repo.as_str(), r.skill.as_str()), ("vercel-labs", "agent-skills", "vercel-react-best-practices"));
        assert_eq!(r.id(), "vercel-labs/agent-skills/vercel-react-best-practices");
        assert_eq!(r.source(), "vercel-labs/agent-skills");
        assert!(SkillRef::parse("vercel/next.js/next-best-practices").is_some(), "repos may have dots");
        // Well-known sources are domains, not GitHub owners.
        assert!(SkillRef::parse("open.feishu.cn/lark-event").is_none());
        assert!(SkillRef::parse("mintlify.com/docs/mintlify").is_none());
        // The skill part becomes a folder name: never a path, always a slug.
        for bad in ["a/b/..", "a/b/.", "a/../c", "a/b/c/d", "a/b", "a//c", "a/b/Has Space", "a/b/x\\y", "a/b/UPPER", "a/b/-x", ""] {
            assert!(SkillRef::parse(bad).is_none(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn slugs_match_the_skills_cli() {
        assert_eq!(skill_slug("React Best_Practices"), "react-best-practices");
        assert_eq!(skill_slug("a.b"), "ab");
        assert_eq!(skill_slug("--a -- b--"), "a-b");
        assert_eq!(skill_slug("Café"), "caf");
        assert_eq!(skill_slug("pdf"), "pdf");
    }

    #[test]
    fn picks_the_folder_named_after_the_skill() {
        let tree = [
            file("README.md", 10),
            file("template/SKILL.md", 10),
            file("examples/pdf/SKILL.md", 10),
            file("skills/pdf/SKILL.md", 10),
            file("skills/pdf/scripts/fill.py", 10),
            file("skills/docx/SKILL.md", 10),
            file("deep/er/still/pdf/SKILL.md", 10),
        ];
        assert_eq!(pick_folder(&tree, "pdf").as_deref(), Some("skills/pdf"), "skills/ wins");
        assert_eq!(pick_folder(&tree, "docx").as_deref(), Some("skills/docx"));
        assert_eq!(pick_folder(&tree[..3], "pdf").as_deref(), Some("examples/pdf"), "else the shallowest");
        assert_eq!(pick_folder(&tree, "xlsx"), None);
        // Folder names are compared as slugs, and SKILL.md in any case.
        assert_eq!(pick_folder(&[file("Skills/My_Tool/skill.md", 1)], "my-tool").as_deref(), Some("Skills/My_Tool"));
    }

    #[test]
    fn a_repository_that_is_one_skill_uses_its_top_level_skill_md() {
        let tree = [file("SKILL.md", 10), file("src/main.rs", 10)];
        assert_eq!(pick_folder(&tree, "anything").as_deref(), Some(""));
        let plan = plan_files(&tree, "").unwrap();
        assert_eq!(plan.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["SKILL.md"], "not the whole repository");
    }

    #[test]
    fn links_are_never_picked_or_planned() {
        let link = TreeFile { link: true, ..file("skills/pdf/SKILL.md", 10) };
        assert_eq!(pick_folder(std::slice::from_ref(&link), "pdf"), None);
        let tree = [file("skills/pdf/SKILL.md", 10), TreeFile { link: true, ..file("skills/pdf/escape", 20) }];
        let plan = plan_files(&tree, "skills/pdf").unwrap();
        assert_eq!(plan.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["SKILL.md"]);
    }

    #[test]
    fn plans_the_skill_folder_without_repo_internals() {
        let tree = [
            file("skills/pdf/SKILL.md", 100),
            TreeFile { executable: true, ..file("skills/pdf/scripts/fill.py", 50) },
            file("skills/pdf/fonts/Inter.ttf", 4000),
            file("skills/pdf/.git/HEAD", 5),
            file("skills/pdf/node_modules/x/index.js", 5),
            file("skills/pdf/scripts/__pycache__/fill.pyc", 5),
            file("skills/pdf-tools/SKILL.md", 5),
            file("skills/docx/SKILL.md", 5),
        ];
        let plan = plan_files(&tree, "skills/pdf").unwrap();
        let paths: Vec<&str> = plan.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["SKILL.md", "scripts/fill.py", "fonts/Inter.ttf"]);
        assert!(plan[1].executable);
        assert_eq!(plan[2].size, 4000);
    }

    #[test]
    fn refuses_folders_without_skill_md_or_too_big() {
        assert!(plan_files(&[file("skills/pdf/README.md", 1)], "skills/pdf").is_err());
        let huge = [file("skills/pdf/SKILL.md", 1), file("skills/pdf/model.bin", 60 * 1024 * 1024)];
        let e = plan_files(&huge, "skills/pdf").unwrap_err().to_string();
        assert!(e.contains("too big"), "{e}");
        let many: Vec<TreeFile> = std::iter::once(file("skills/pdf/SKILL.md", 1)).chain((0..600).map(|i| file(&format!("skills/pdf/f{i}.md"), 1))).collect();
        assert!(plan_files(&many, "skills/pdf").is_err());
    }

    #[test]
    fn writes_the_skill_with_a_marker_and_leaves_no_temp_folder() {
        let t = tempfile::tempdir().unwrap();
        let skills = t.path().join("shared").join("skills");
        let target = write_skill(&skills, &pdf(), &download(&[("SKILL.md", "---\nname: pdf\n---\n"), ("scripts/fill.py", "print(1)")])).unwrap();
        assert_eq!(target, skills.join("pdf"));
        assert_eq!(fs::read_to_string(target.join("scripts").join("fill.py")).unwrap(), "print(1)");
        assert_eq!(installed_id(&target).as_deref(), Some("anthropics/skills/pdf"));
        let leftovers: Vec<_> = fs::read_dir(t.path().join("shared")).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect();
        assert_eq!(leftovers, ["skills"], "only the skills folder: {leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_scripts_executable() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let mut d = download(&[("SKILL.md", "s"), ("run.sh", "echo")]);
        d.files[1].0.executable = true;
        let target = write_skill(t.path(), &pdf(), &d).unwrap();
        assert_eq!(fs::metadata(target.join("run.sh")).unwrap().permissions().mode() & 0o111, 0o111);
        assert_eq!(fs::metadata(target.join("SKILL.md")).unwrap().permissions().mode() & 0o111, 0);
    }

    #[test]
    fn never_overwrites_a_skill_that_is_already_there() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("pdf")).unwrap();
        fs::write(t.path().join("pdf").join("SKILL.md"), "mine").unwrap();
        assert!(write_skill(t.path(), &pdf(), &download(&[("SKILL.md", "theirs")])).is_err());
        assert_eq!(fs::read_to_string(t.path().join("pdf").join("SKILL.md")).unwrap(), "mine");
        let names: Vec<_> = fs::read_dir(t.path()).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect();
        assert_eq!(names, ["pdf"], "no temp folder left: {names:?}");
    }

    #[test]
    fn removes_only_what_switchyard_installed() {
        let t = tempfile::tempdir().unwrap();
        // Someone else's skill with the same name stays.
        fs::create_dir_all(t.path().join("pdf")).unwrap();
        fs::write(t.path().join("pdf").join("SKILL.md"), "mine").unwrap();
        assert!(remove_skill(t.path(), &pdf()).is_err());
        assert!(t.path().join("pdf").join("SKILL.md").exists());
        fs::remove_dir_all(t.path().join("pdf")).unwrap();

        // A copy installed from another repository stays too.
        let other = SkillRef::parse("someone/fork/pdf").unwrap();
        write_skill(t.path(), &other, &download(&[("SKILL.md", "fork")])).unwrap();
        assert!(remove_skill(t.path(), &pdf()).is_err());
        assert!(t.path().join("pdf").exists());
        fs::remove_dir_all(t.path().join("pdf")).unwrap();

        // Ours goes.
        write_skill(t.path(), &pdf(), &download(&[("SKILL.md", "s")])).unwrap();
        remove_skill(t.path(), &pdf()).unwrap();
        assert!(!t.path().join("pdf").exists());
        assert!(remove_skill(t.path(), &pdf()).is_err(), "nothing left to remove");
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_a_link_when_removing() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real");
        write_skill(&real, &pdf(), &download(&[("SKILL.md", "s")])).unwrap();
        let skills = t.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        std::os::unix::fs::symlink(real.join("pdf"), skills.join("pdf")).unwrap();
        assert!(remove_skill(&skills, &pdf()).is_err());
        assert!(real.join("pdf").join("SKILL.md").exists(), "the link's target is untouched");
    }

    #[test]
    fn installs_into_each_tools_first_skills_folder() {
        let shared = Path::new("/shared/gemini");
        let gemini = crate::providers::get(crate::model::ProviderId::Gemini);
        assert_eq!(skills_dir(shared, gemini.as_ref()), Some(shared.join(".gemini/skills")));
        let claude = crate::providers::get(crate::model::ProviderId::Claude);
        assert_eq!(skills_dir(Path::new("/shared/claude"), claude.as_ref()), Some(PathBuf::from("/shared/claude/skills")));
    }
}
