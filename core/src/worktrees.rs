//! Git worktree scanner: every linked worktree of every repository found in
//! the scan, with what it costs on disk and whether it still holds work.
//!
//! The source of truth is each main repository's `.git/worktrees/<name>/`
//! admin folder, so worktrees outside the scanned root, and ones whose folder
//! was deleted by hand (prunable), are found too. Read-only: it suggests
//! commands but never runs anything that changes a repository.

use crate::markers::{GIT_DIR, GIT_FILE};
use crate::projects::{self, Project};
use crate::DirNode;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Touched within this many days: in use, never suggested for removal.
pub const ACTIVE_DAYS: i64 = crate::plan::ACTIVE_DAYS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Folder is gone but git still lists it: `git worktree prune` tidies it.
    Prunable,
    /// `git worktree lock`ed: someone asked git to keep it.
    Locked,
    /// Uncommitted or untracked changes: removing would lose work.
    Dirty,
    /// Touched recently.
    Active,
    /// Clean, idle, and its branch is merged into the base branch or its
    /// upstream branch was deleted: the work is done.
    Done,
    /// Clean and idle, but its branch has commits not in the base branch.
    /// Removing the worktree keeps the branch; review first.
    Idle,
    /// Git state unknown (git missing or timed out).
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    /// Main repository.
    pub repo: PathBuf,
    /// Admin folder name under `.git/worktrees/`.
    pub name: String,
    /// Checked-out branch; None when detached.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub exists: bool,
    pub locked: bool,
    /// Allocated bytes of the whole worktree; None when outside the scan.
    pub size: Option<u64>,
    /// Build folders inside it (node_modules, target/, ...).
    pub artifact_bytes: u64,
    /// Newest mtime of its own files (artifacts and .git excluded).
    pub last_active: Option<i64>,
    pub dirty: Option<bool>,
    /// Branch the merge check compares against (origin/HEAD or the main worktree's branch).
    pub base: Option<String>,
    pub merged: Option<bool>,
    pub upstream_gone: bool,
    /// Tool that made it, guessed from its location.
    pub created_by: Option<&'static str>,
    pub status: Status,
    /// Command that would remove (or prune) it. Shown, never run here.
    pub suggested: Option<Vec<String>>,
}

/// Where agent tools put their worktrees (path fragment, tool). Heuristic.
const CREATORS: &[(&str, &str)] = &[
    ("/.claude/worktrees/", "Claude Code"),
    ("/conductor/workspaces/", "Conductor"),
    ("/.codex/worktrees/", "Codex"),
    ("/.cursor/worktrees/", "Cursor"),
];

fn created_by(p: &Path) -> Option<&'static str> {
    let s = p.to_string_lossy();
    CREATORS
        .iter()
        .find(|(frag, _)| s.contains(frag))
        .map(|(_, t)| *t)
}

/// Main repositories found in the tree: `.git` folders, plus the main repo of
/// any linked worktree (which may live outside the scanned root).
fn find_repos(node: &DirNode, path: &Path, out: &mut BTreeSet<PathBuf>) {
    if node.markers & GIT_DIR != 0 {
        out.insert(path.to_path_buf());
    }
    if node.markers & GIT_FILE != 0 {
        if let Some(main) = projects::worktree_main(path) {
            out.insert(main);
        }
    }
    for c in &node.children {
        if projects::NEVER_DESCEND.contains(&c.name.as_str()) || c.name.ends_with(".app") {
            continue;
        }
        find_repos(c, &path.join(&c.name), out);
    }
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Linked worktrees registered in `repo/.git/worktrees/`.
fn registered(repo: &Path) -> Vec<Worktree> {
    let admin = repo.join(".git/worktrees");
    let Ok(rd) = std::fs::read_dir(&admin) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let dir = e.path();
        let Some(gitdir) = read_trim(&dir.join("gitdir")) else {
            continue;
        };
        // gitdir holds `<worktree>/.git`; relative paths are relative to the admin folder.
        let gitfile = dir.join(&gitdir);
        let Some(path) = gitfile.parent().map(Path::to_path_buf) else {
            continue;
        };
        let head = read_trim(&dir.join("HEAD"));
        let (branch, head) = match head.as_deref().and_then(|h| h.strip_prefix("ref: refs/heads/")) {
            Some(b) => (Some(b.to_string()), None),
            None => (None, head.map(|h| h[..h.len().min(8)].to_string())),
        };
        out.push(Worktree {
            exists: gitfile.is_file(),
            locked: dir.join("locked").exists(),
            name: e.file_name().to_string_lossy().into_owned(),
            created_by: created_by(&path),
            repo: repo.to_path_buf(),
            path,
            branch,
            head,
            size: None,
            artifact_bytes: 0,
            last_active: None,
            dirty: None,
            base: None,
            merged: None,
            upstream_gone: false,
            status: Status::Unknown,
            suggested: None,
        });
    }
    out
}

struct RepoInfo {
    base: Option<String>,
    merged: Option<HashSet<String>>,
    gone: HashSet<String>,
}

fn git_lines(git: &Path, repo: &Path, args: &[&str]) -> Option<Vec<String>> {
    let r = repo.display().to_string();
    let mut a = vec!["-C", r.as_str()];
    a.extend_from_slice(args);
    projects::run(git, &a, "", Duration::from_secs(20))
        .filter(|(code, _)| *code == 0)
        .map(|(_, out)| out.lines().map(|l| l.trim().to_string()).collect())
}

fn repo_info(git: &Path, repo: &Path) -> RepoInfo {
    let base = git_lines(git, repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .and_then(|l| l.into_iter().next())
        .filter(|b| !b.is_empty())
        .or_else(|| projects::head_branch(repo).filter(|b| !b.starts_with("detached")));
    let merged = base.as_deref().and_then(|b| {
        git_lines(git, repo, &["branch", "--format=%(refname:short)", "--merged", b])
            .map(|l| l.into_iter().collect())
    });
    let gone = git_lines(
        git,
        repo,
        &["for-each-ref", "--format=%(refname:short)\t%(upstream:track)", "refs/heads"],
    )
    .unwrap_or_default()
    .into_iter()
    .filter_map(|l| {
        let (b, track) = l.split_once('\t')?;
        (track == "[gone]").then(|| b.to_string())
    })
    .collect();
    RepoInfo { base, merged, gone }
}

fn classify(w: &Worktree, now: i64) -> Status {
    if !w.exists {
        return Status::Prunable;
    }
    if w.locked {
        return Status::Locked;
    }
    match w.dirty {
        Some(true) => return Status::Dirty,
        None => return Status::Unknown,
        Some(false) => {}
    }
    if w.last_active.is_some_and(|t| (now - t) / 86_400 < ACTIVE_DAYS) {
        return Status::Active;
    }
    if w.merged == Some(true) || w.upstream_gone {
        Status::Done
    } else {
        Status::Idle
    }
}

/// Find and describe every linked worktree. `tree` is the scan of `root`;
/// `projects` supplies build-folder sizes. `git` None skips git queries
/// (statuses are then Prunable, Locked or Unknown).
pub fn detect(
    tree: &DirNode,
    root: &Path,
    projects: &[Project],
    git: Option<&Path>,
    now: i64,
) -> Vec<Worktree> {
    use rayon::prelude::*;
    let mut repos = BTreeSet::new();
    find_repos(tree, root, &mut repos);
    let repos: Vec<PathBuf> = repos.into_iter().collect();

    let mut wts: Vec<Worktree> = repos.iter().flat_map(|r| registered(r)).collect();
    if wts.is_empty() {
        return wts;
    }

    let info: HashMap<PathBuf, RepoInfo> = match git {
        Some(g) => {
            let with_wts: HashSet<&PathBuf> = wts.iter().map(|w| &w.repo).collect();
            with_wts
                .into_par_iter()
                .map(|r| (r.clone(), repo_info(g, r)))
                .collect()
        }
        None => HashMap::new(),
    };

    wts.par_iter_mut().for_each(|w| {
        if w.exists {
            if let Some(node) = w.path.strip_prefix(root).ok().and_then(|rel| tree.find(rel)) {
                w.size = Some(node.size);
                w.last_active = Some(projects::active_mtime(node));
            }
            w.artifact_bytes = projects
                .iter()
                .filter(|p| p.path.starts_with(&w.path))
                .map(|p| p.artifact_bytes)
                .sum();
            if let Some(g) = git {
                // --ignored=no: build folders don't count as work.
                w.dirty = git_lines(g, &w.path, &["status", "--porcelain", "--ignored=no"])
                    .map(|l| l.iter().any(|x| !x.is_empty()));
            }
        }
        if let Some(i) = info.get(&w.repo) {
            w.base = i.base.clone();
            if let Some(b) = &w.branch {
                w.merged = i.merged.as_ref().map(|m| m.contains(b));
                w.upstream_gone = i.gone.contains(b);
            }
        }
        w.status = classify(w, now);
        let repo = w.repo.display().to_string();
        w.suggested = match w.status {
            Status::Prunable => Some(vec!["git".into(), "-C".into(), repo, "worktree".into(), "prune".into()]),
            Status::Done | Status::Idle => Some(vec![
                "git".into(),
                "-C".into(),
                repo,
                "worktree".into(),
                "remove".into(),
                w.path.display().to_string(),
            ]),
            _ => None,
        };
    });
    wts.sort_by_key(|w| std::cmp::Reverse(w.size.unwrap_or(0)));
    wts
}
