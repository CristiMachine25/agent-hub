//! Skills from the public skills.sh directory, installed into each tool's shared folder so every
//! account that links its skills gets them at once. The files come from the skill's GitHub
//! repository at one commit. Nothing is ever overwritten, and only folders Agent Hub installed
//! (they carry a marker file) are ever removed.
use crate::model::{DirectorySkill, ShareKind, SkillPreview};
use crate::providers::ProviderAdapter;
use crate::{knowledge, sharing, util};
use anyhow::{anyhow, bail, Context};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Written into every skill folder Agent Hub installs: where it came from.
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
        let skill_ok = !skill.is_empty() && skill.len() <= 100 && skill_slug(skill) == *skill && (!cfg!(windows) || sharing::windows_safe_name(skill));
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

/// The SKILL.md files of a repository, best first: under skills/, then the shallowest.
pub fn skill_md_files(files: &[TreeFile]) -> Vec<&TreeFile> {
    let mut found: Vec<&TreeFile> = files.iter().filter(|f| !f.link && is_skill_md(&f.path)).collect();
    found.sort_by_key(|f| (!f.path.starts_with("skills/"), f.path.matches('/').count(), f.path.as_str()));
    found
}

/// The folder of the first SKILL.md whose name is `skill` as a slug. `read` pairs each folder
/// ("" for the top of the repository) with its SKILL.md.
pub fn pick_by_name(read: &[(String, String)], skill: &str) -> Option<String> {
    read.iter().find(|(_, md)| front_matter(md).0.is_some_and(|name| skill_slug(&name) == skill)).map(|(dir, _)| dir.clone())
}

/// Where `skill` lives in a repository: the best folder named after it, as the skill spec asks.
/// When none is, the directory named the skill after its SKILL.md: see pick_by_name.
pub fn pick_folder(files: &[TreeFile], skill: &str) -> Option<String> {
    skill_md_files(files)
        .into_iter()
        .filter_map(|f| f.path.rsplit_once('/').map(|(dir, _)| dir))
        .find(|dir| dir.rsplit('/').next().is_some_and(|name| skill_slug(name) == skill))
        .map(str::to_string)
}

fn is_skill_md(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
}

/// The files of the skill folder to download, relative to it, without links, repo internals and
/// installed packages. A repository that is one skill brings only its SKILL.md: the rest of the
/// repository is not part of the skill.
pub fn plan_files(files: &[TreeFile], folder: &str) -> anyhow::Result<Vec<TreeFile>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
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
            bail!("The skill has a file Agent Hub can't save safely: {rel}");
        }
        // On macOS and Windows one of the two would silently replace the other.
        if !seen.insert(rel.to_lowercase()) {
            bail!("The skill has two files whose names differ only in case: {rel}");
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
        bail!("A skill named {} is already in the shared folder", r.skill);
    }
    fs::create_dir_all(skills_dir).with_context(|| format!("creating {}", skills_dir.display()))?;
    let tmp = skills_dir.parent().unwrap_or(skills_dir).join(format!(".switchyard-install-{}", uuid::Uuid::new_v4().simple()));
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

/// The directory id of a skill folder Agent Hub installed, from its marker.
pub fn installed_id(dir: &Path) -> Option<String> {
    util::read_json(&dir.join(MARKER))?.get("id")?.as_str().map(str::to_string)
}

/// Delete `<skills_dir>/<skill>` when Agent Hub installed it from `r`. A link, or a folder that
/// someone else put there, is left alone.
pub fn remove_skill(skills_dir: &Path, r: &SkillRef) -> anyhow::Result<()> {
    let target = skills_dir.join(&r.skill);
    let meta = fs::symlink_metadata(&target).map_err(|_| anyhow!("It isn't installed"))?;
    if !meta.is_dir() || sharing::is_link(&target) || installed_id(&target).as_deref() != Some(r.id().as_str()) {
        bail!("Agent Hub didn't install this copy, so it was left alone");
    }
    fs::remove_dir_all(&target).with_context(|| format!("deleting {}", target.display()))
}

/// The folder a tool reads skills from inside its shared folder: its first skills path.
pub fn skills_dir(shared: &Path, adapter: &dyn ProviderAdapter) -> Option<PathBuf> {
    let (_, rels) = adapter.shareable_paths().into_iter().find(|(k, _)| *k == ShareKind::Skills)?;
    rels.first().map(|rel| shared.join(rel))
}

/// Search results that can be installed (GitHub skills), in the directory's order, without repeats.
pub fn parse_search(v: &Value) -> Vec<DirectorySkill> {
    let mut seen = HashSet::new();
    let found = v.get("skills").and_then(Value::as_array).into_iter().flatten();
    found
        .filter_map(|s| {
            let r = SkillRef::parse(s.get("id")?.as_str()?)?;
            let name = s.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&r.skill).to_string();
            let installs = s.get("installs").and_then(Value::as_u64).unwrap_or(0);
            seen.insert(r.id()).then(|| DirectorySkill { id: r.id(), name, source: r.source(), installs })
        })
        .collect()
}

/// A recursive GitHub tree listing as files. Submodules and folders are left out.
pub fn parse_tree(v: &Value) -> anyhow::Result<Vec<TreeFile>> {
    if v.get("truncated").and_then(Value::as_bool) == Some(true) {
        bail!("This repository is too big to list, so the skill can't be installed from here");
    }
    let tree = v.get("tree").and_then(Value::as_array).ok_or_else(|| anyhow!("GitHub didn't list the repository's files"))?;
    Ok(tree
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("blob"))
        .filter_map(|e| {
            let mode = e.get("mode")?.as_str()?;
            Some(TreeFile {
                path: e.get("path")?.as_str()?.to_string(),
                size: e.get("size").and_then(Value::as_u64).unwrap_or(0),
                executable: mode == "100755",
                link: mode == "120000",
            })
        })
        .collect())
}

/// (name, description, body) of a SKILL.md. Only plain, quoted and folded (`>`, `|`) values are
/// read: enough for the two fields every skill has.
pub fn front_matter(md: &str) -> (Option<String>, Option<String>, &str) {
    let Some(rest) = md.strip_prefix("---").and_then(|r| r.strip_prefix("\r\n").or_else(|| r.strip_prefix('\n'))) else {
        return (None, None, md);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let head = &rest[..offset];
            let body = rest[offset + line.len()..].trim_start_matches(['\r', '\n']);
            return (field(head, "name"), field(head, "description"), body);
        }
        offset += line.len();
    }
    (None, None, md)
}

fn field(head: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = head.lines().collect();
    let at = lines.iter().position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))?;
    let value = lines[at][key.len() + 1..].trim();
    let text = if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
        let block = lines[at + 1..].iter().take_while(|l| l.starts_with([' ', '\t']) || l.trim().is_empty());
        block.map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
    } else {
        ['"', '\''].iter().find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q))).unwrap_or(value).to_string()
    };
    (!text.is_empty()).then_some(text)
}

/// The skill folders in `skills_dir`, with the directory id of those Agent Hub installed.
pub fn list_installed(skills_dir: &Path) -> Vec<(String, Option<String>)> {
    let Ok(entries) = fs::read_dir(skills_dir) else { return Vec::new() };
    let mut out: Vec<(String, Option<String>)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok().filter(|n| !n.starts_with('.')).map(|n| (n, installed_id(&e.path()))))
        .collect();
    out.sort();
    out
}

/// Add a downloaded skill to one tool's shared folder, seeding that folder first if it never was.
pub fn install_into(shared: &Path, adapter: &dyn ProviderAdapter, r: &SkillRef, d: &Download) -> anyhow::Result<PathBuf> {
    knowledge::ensure_seeded(shared, adapter).with_context(|| format!("preparing {}", shared.display()))?;
    write_skill(&tool_skills_dir(shared, adapter)?, r, d)
}

/// Remove a skill Agent Hub installed from one tool's shared folder.
pub fn remove_from(shared: &Path, adapter: &dyn ProviderAdapter, r: &SkillRef) -> anyhow::Result<()> {
    remove_skill(&tool_skills_dir(shared, adapter)?, r)
}

fn tool_skills_dir(shared: &Path, adapter: &dyn ProviderAdapter) -> anyhow::Result<PathBuf> {
    skills_dir(shared, adapter).ok_or_else(|| anyhow!("This tool has no skills folder"))
}

// ---------- network ----------

const DIRECTORY: &str = "https://skills.sh";

/// Files downloaded at the same time.
const PARALLEL: usize = 8;

/// Most SKILL.md files read to find a skill by its name, and the largest one read.
const MAX_NAME_LOOKUPS: usize = 200;
const MAX_NAME_LOOKUP_BYTES: u64 = 256 * 1024;

/// A skill found in its repository at one commit: the files install writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub commit: String,
    /// The skill's folder in the repository ("" for a repository that is one skill).
    pub folder: String,
    pub files: Vec<TreeFile>,
}

/// Where each previewed skill was found, so install writes the commit the user read rather than
/// whatever HEAD points at by then.
static REVIEWED: Mutex<BTreeMap<String, Located>> = Mutex::new(BTreeMap::new());

fn remember(r: &SkillRef, at: &Located) {
    REVIEWED.lock().unwrap_or_else(|e| e.into_inner()).insert(r.id(), at.clone());
}

fn reviewed(r: &SkillRef) -> Option<Located> {
    REVIEWED.lock().unwrap_or_else(|e| e.into_inner()).get(&r.id()).cloned()
}

/// The directory's search, best matches first. Queries need at least two characters.
pub async fn search(query: &str) -> anyhow::Result<Vec<DirectorySkill>> {
    let url = reqwest::Url::parse_with_params(&format!("{DIRECTORY}/api/search"), &[("q", query), ("limit", "50")])?;
    let v: Value = send(util::http().get(url)).await?.json().await?;
    Ok(parse_search(&v))
}

/// The skill's SKILL.md to read before installing, from its GitHub repository at the commit HEAD
/// points at now. Installing it afterwards writes that same commit's files.
pub async fn preview(r: &SkillRef) -> anyhow::Result<SkillPreview> {
    let http = util::http();
    let at = locate(&http, r).await?;
    let md = at.files.iter().find(|f| f.path.eq_ignore_ascii_case("SKILL.md")).cloned().ok_or_else(|| anyhow!("The skill has no SKILL.md"))?;
    let (_, bytes) = fetch_raw(http.clone(), r.clone(), at.commit.clone(), at.folder.clone(), md).await?;
    let md = String::from_utf8_lossy(&bytes);
    let (name, description, body) = front_matter(&md);
    let preview = SkillPreview {
        id: r.id(),
        name: name.unwrap_or_else(|| r.skill.clone()),
        description: description.unwrap_or_default(),
        body: body.to_string(),
        files: at.files.len(),
    };
    remember(r, &at);
    Ok(preview)
}

/// The skill's files from its GitHub repository: from the commit its preview showed, or from the
/// commit HEAD points at now when it wasn't previewed.
pub async fn download(r: &SkillRef) -> anyhow::Result<Download> {
    let http = util::download_http();
    let at = match reviewed(r) {
        Some(at) => at,
        None => locate(&http, r).await?,
    };
    let got = fetch_all(&http, r, &at.commit, &at.folder, at.files).await?;
    Ok(Download { commit: at.commit, folder: at.folder, files: got })
}

/// SKILL.md files small enough to read for their name, best first.
fn name_lookups(files: &[TreeFile]) -> Vec<TreeFile> {
    skill_md_files(files).into_iter().filter(|f| f.size <= MAX_NAME_LOOKUP_BYTES).take(MAX_NAME_LOOKUPS).cloned().collect()
}

/// The skill's folder and files at the commit HEAD points at now.
async fn locate(http: &reqwest::Client, r: &SkillRef) -> anyhow::Result<Located> {
    let api = format!("https://api.github.com/repos/{}/{}", r.owner, r.repo);
    let head = http.get(format!("{api}/commits/HEAD")).header(reqwest::header::ACCEPT, "application/vnd.github.sha");
    let commit = send(head).await?.text().await?.trim().to_string();
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("GitHub didn't say which version of {} to download", r.source());
    }
    let listing = http.get(format!("{api}/git/trees/{commit}?recursive=1")).header(reqwest::header::ACCEPT, "application/vnd.github+json");
    let files = parse_tree(&send(listing).await?.json().await?)?;
    let folder = match pick_folder(&files, &r.skill) {
        Some(folder) => folder,
        None => {
            let read = fetch_all(http, r, &commit, "", name_lookups(&files)).await?;
            let read: Vec<(String, String)> = read
                .into_iter()
                .map(|(f, bytes)| (f.path.rsplit_once('/').map_or("", |(dir, _)| dir).to_string(), String::from_utf8_lossy(&bytes).into_owned()))
                .collect();
            pick_by_name(&read, &r.skill).ok_or_else(|| anyhow!("{} has no skill named {}", r.source(), r.skill))?
        }
    };
    let files = plan_files(&files, &folder)?;
    Ok(Located { commit, folder, files })
}

/// Download `files` (relative to `folder`) at `commit`, a few at a time, sorted by path.
async fn fetch_all(http: &reqwest::Client, r: &SkillRef, commit: &str, folder: &str, files: Vec<TreeFile>) -> anyhow::Result<Vec<(TreeFile, Vec<u8>)>> {
    let mut pending = files.into_iter();
    let mut running = tokio::task::JoinSet::new();
    let mut got = Vec::new();
    loop {
        while running.len() < PARALLEL {
            let Some(f) = pending.next() else { break };
            running.spawn(fetch_raw(http.clone(), r.clone(), commit.to_string(), folder.to_string(), f));
        }
        match running.join_next().await {
            Some(done) => got.push(done??),
            None => break,
        }
    }
    got.sort_by(|a: &(TreeFile, Vec<u8>), b| a.0.path.cmp(&b.0.path));
    Ok(got)
}

async fn fetch_raw(http: reqwest::Client, r: SkillRef, commit: String, folder: String, f: TreeFile) -> anyhow::Result<(TreeFile, Vec<u8>)> {
    let mut url = reqwest::Url::parse("https://raw.githubusercontent.com/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("bad download url"))?
        .extend([r.owner.as_str(), r.repo.as_str(), commit.as_str()])
        .extend(folder.split('/').filter(|s| !s.is_empty()))
        .extend(f.path.split('/'));
    let bytes = send(http.get(url)).await?.bytes().await?;
    if bytes.len() as u64 != f.size {
        bail!("{} didn't download completely", f.path);
    }
    Ok((f, bytes.to_vec()))
}

/// Send a GET and turn failures into sentences: offline, GitHub's hourly limit, not found.
async fn send(req: reqwest::RequestBuilder) -> anyhow::Result<reqwest::Response> {
    let resp = req.send().await.map_err(|e| {
        let host = e.url().and_then(|u| u.host_str()).unwrap_or("the server").to_string();
        anyhow!("Couldn't reach {host}{}", if e.is_timeout() { ": it took too long to answer" } else { ". Check your connection." })
    })?;
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let host = resp.url().host_str().unwrap_or_default().to_string();
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    if host == "api.github.com" && (status.as_u16() == 429 || header("x-ratelimit-remaining").as_deref() == Some("0")) {
        let reset = header("x-ratelimit-reset").and_then(|s| s.parse::<i64>().ok()).and_then(|s| chrono::DateTime::from_timestamp(s, 0));
        let when = reset.map(|t| format!(" Try again at {}.", util::local_clock(t, chrono::Utc::now()))).unwrap_or_default();
        bail!("GitHub's hourly download limit for this network is used up.{when}");
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("Not found on {host}. The skill or its repository may have moved.");
    }
    bail!("{host} answered {status}")
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
    fn falls_back_to_the_name_in_skill_md() {
        // vercel-labs/agent-skills keeps vercel-react-best-practices in skills/react-best-practices.
        let tree = [file("skills/react-best-practices/SKILL.md", 10), file("skills/x/skill.md", 10), file("README.md", 1)];
        assert_eq!(pick_folder(&tree, "vercel-react-best-practices"), None);
        let mds: Vec<&str> = skill_md_files(&tree).iter().map(|f| f.path.as_str()).collect();
        assert_eq!(mds, ["skills/react-best-practices/SKILL.md", "skills/x/skill.md"]);
        let read = [
            ("skills/x".to_string(), "---\nname: other\n---\n".to_string()),
            ("skills/react-best-practices".to_string(), "---\nname: Vercel React Best Practices\ndescription: d\n---\n".to_string()),
        ];
        assert_eq!(pick_by_name(&read, "vercel-react-best-practices").as_deref(), Some("skills/react-best-practices"));
        assert_eq!(pick_by_name(&read, "missing"), None);
    }

    #[test]
    fn a_repository_that_is_one_skill_is_found_by_its_name() {
        let tree = [file("SKILL.md", 10), file("src/main.rs", 10)];
        assert_eq!(pick_folder(&tree, "my-tool"), None, "a top-level SKILL.md has no folder name to match");
        assert_eq!(pick_by_name(&[(String::new(), "---\nname: my_tool\n---".into())], "my-tool").as_deref(), Some(""));
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
    fn refuses_files_that_differ_only_in_case() {
        let tree = [file("skills/pdf/SKILL.md", 1), file("skills/pdf/skill.md", 1)];
        let e = plan_files(&tree, "skills/pdf").unwrap_err().to_string();
        assert!(e.contains("only in case"), "{e}");
        assert!(plan_files(&[file("skills/pdf/SKILL.md", 1), file("skills/pdf/Scripts/a.sh", 1), file("skills/pdf/scripts/b.sh", 1)], "skills/pdf").is_ok());
    }

    #[test]
    fn windows_device_names_are_not_skill_folders() {
        assert_eq!(SkillRef::parse("a/b/con").is_some(), !cfg!(windows));
        assert_eq!(SkillRef::parse("a/b/com1").is_some(), !cfg!(windows));
        assert!(SkillRef::parse("a/b/console").is_some());
    }

    #[test]
    fn only_small_skill_md_files_are_read_for_their_name() {
        let tree = [file("skills/a/SKILL.md", 1000), file("skills/huge/SKILL.md", 10 * 1024 * 1024), file("b/SKILL.md", 10)];
        let paths: Vec<String> = name_lookups(&tree).into_iter().map(|f| f.path).collect();
        assert_eq!(paths, ["skills/a/SKILL.md", "b/SKILL.md"]);
    }

    #[test]
    fn install_uses_the_commit_the_preview_showed() {
        let r = SkillRef::parse("someone/pinned/tool").unwrap();
        assert_eq!(reviewed(&r), None);
        let at = Located { commit: "b".repeat(40), folder: "skills/tool".into(), files: vec![file("SKILL.md", 3)] };
        remember(&r, &at);
        assert_eq!(reviewed(&r), Some(at));
        assert_eq!(reviewed(&pdf()), None, "kept per skill");
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
    fn removes_only_what_agent_hub_installed() {
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

    #[test]
    fn keeps_installable_search_results_once() {
        let v = serde_json::json!({ "query": "pdf", "skills": [
            { "id": "anthropics/skills/pdf", "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 120 },
            { "id": "open.feishu.cn/lark-pdf", "source": "open.feishu.cn", "skillId": "lark-pdf", "name": "lark-pdf", "installs": 90 },
            { "id": "anthropics/skills/pdf", "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 120 },
            { "id": "someone/tools/pdf-tools", "name": "PDF Tools" },
            { "name": "no id" }
        ]});
        let found = parse_search(&v);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], DirectorySkill { id: "anthropics/skills/pdf".into(), name: "pdf".into(), source: "anthropics/skills".into(), installs: 120 });
        assert_eq!((found[1].name.as_str(), found[1].source.as_str(), found[1].installs), ("PDF Tools", "someone/tools", 0));
        assert!(parse_search(&serde_json::json!({ "error": "Query must be at least 2 characters" })).is_empty());
    }

    #[test]
    fn reads_github_trees() {
        let v = serde_json::json!({ "sha": "t", "truncated": false, "tree": [
            { "path": "skills", "mode": "040000", "type": "tree" },
            { "path": "skills/pdf/SKILL.md", "mode": "100644", "type": "blob", "size": 12 },
            { "path": "skills/pdf/run.sh", "mode": "100755", "type": "blob", "size": 3 },
            { "path": "skills/pdf/latest", "mode": "120000", "type": "blob", "size": 9 },
            { "path": "vendor/lib", "mode": "160000", "type": "commit" }
        ]});
        let files = parse_tree(&v).unwrap();
        assert_eq!(
            files,
            [
                TreeFile { path: "skills/pdf/SKILL.md".into(), size: 12, executable: false, link: false },
                TreeFile { path: "skills/pdf/run.sh".into(), size: 3, executable: true, link: false },
                TreeFile { path: "skills/pdf/latest".into(), size: 9, executable: false, link: true },
            ]
        );
        let cut = serde_json::json!({ "truncated": true, "tree": [] });
        assert!(parse_tree(&cut).unwrap_err().to_string().contains("too big"));
        assert!(parse_tree(&serde_json::json!({ "message": "Not Found" })).is_err());
    }

    #[test]
    fn reads_skill_front_matter() {
        let md = "---\nname: pdf\ndescription: \"Read, merge and split PDFs.\"\nlicense: MIT\n---\n\n# PDF\nBody";
        assert_eq!(front_matter(md), (Some("pdf".into()), Some("Read, merge and split PDFs.".into()), "# PDF\nBody"));
        let folded = "---\r\nname: 'x'\r\ndescription: >\r\n  Two lines\r\n  of text.\r\nmetadata:\r\n  a: b\r\n---\r\nHi";
        assert_eq!(front_matter(folded), (Some("x".into()), Some("Two lines of text.".into()), "Hi"));
        assert_eq!(front_matter("# No front matter"), (None, None, "# No front matter"));
        assert_eq!(front_matter("---\nname: open"), (None, None, "---\nname: open"), "unclosed front matter is body");
    }

    #[test]
    fn lists_installed_skill_folders() {
        let t = tempfile::tempdir().unwrap();
        assert!(list_installed(&t.path().join("missing")).is_empty());
        write_skill(t.path(), &pdf(), &download(&[("SKILL.md", "s")])).unwrap();
        fs::create_dir_all(t.path().join("mine")).unwrap();
        fs::create_dir_all(t.path().join(".hidden")).unwrap();
        fs::write(t.path().join("notes.txt"), "x").unwrap();
        assert_eq!(list_installed(t.path()), [("mine".to_string(), None), ("pdf".to_string(), Some("anthropics/skills/pdf".to_string()))]);
    }

    #[test]
    fn installs_into_and_removes_from_a_tools_shared_folder() {
        let t = tempfile::tempdir().unwrap();
        // Already seeded, so nothing is copied from this machine's real ~/.claude.
        fs::write(t.path().join(crate::knowledge::SEEDED_MARKER), "x").unwrap();
        let claude = crate::providers::get(crate::model::ProviderId::Claude);
        let target = install_into(t.path(), claude.as_ref(), &pdf(), &download(&[("SKILL.md", "s")])).unwrap();
        assert_eq!(target, t.path().join("skills").join("pdf"));
        let e = install_into(t.path(), claude.as_ref(), &pdf(), &download(&[("SKILL.md", "s")])).unwrap_err();
        assert_eq!(e.to_string(), "A skill named pdf is already in the shared folder");
        remove_from(t.path(), claude.as_ref(), &pdf()).unwrap();
        assert!(!target.exists());
    }

    /// Talks to skills.sh and GitHub: `cargo test skills::tests::live -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_search_preview_and_install() {
        let found = search("pdf").await.unwrap();
        assert!(found.iter().any(|s| s.id == "anthropics/skills/pdf"), "{found:?}");
        let shown = preview(&pdf()).await.unwrap();
        assert_eq!(shown.name, "pdf");
        assert!(!shown.description.is_empty() && shown.files > 1);
        let d = super::download(&pdf()).await.unwrap();
        assert_eq!(d.folder, "skills/pdf");
        let renamed = SkillRef::parse("vercel-labs/agent-skills/vercel-react-best-practices").unwrap();
        assert_eq!(super::download(&renamed).await.unwrap().folder, "skills/react-best-practices");
        let t = tempfile::tempdir().unwrap();
        let target = write_skill(t.path(), &pdf(), &d).unwrap();
        assert!(target.join("SKILL.md").is_file());
        assert_eq!(installed_id(&target).as_deref(), Some("anthropics/skills/pdf"));
    }
}
