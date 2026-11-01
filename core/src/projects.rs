//! Project build-artifact detector (PLAN.md §3.1), modelled on kondo's marker
//! table. Runs over the scan tree: a directory with a marker file is a
//! project, and matching child directories are its artifacts.

use crate::catalog::Safety;
use crate::markers::*;
use crate::DirNode;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// (marker bits, artifact dir name, safety, what regenerates it)
const TABLE: &[(u32, &str, Safety, &str)] = &[
    (
        PACKAGE_JSON,
        "node_modules",
        Safety::Regenerable,
        "npm/yarn/pnpm/bun install",
    ),
    (
        PACKAGE_JSON,
        ".next",
        Safety::Regenerable,
        "next build / next dev",
    ),
    (PACKAGE_JSON, ".turbo", Safety::Regenerable, "turbo run"),
    (
        PACKAGE_JSON,
        ".parcel-cache",
        Safety::Regenerable,
        "parcel build",
    ),
    (PACKAGE_JSON, ".angular", Safety::Regenerable, "ng build"),
    (PACKAGE_JSON, ".nuxt", Safety::Regenerable, "nuxt build"),
    (
        PACKAGE_JSON,
        ".svelte-kit",
        Safety::Regenerable,
        "vite build / dev",
    ),
    (PACKAGE_JSON, ".expo", Safety::Regenerable, "expo start"),
    (
        PACKAGE_JSON,
        "dist",
        Safety::Review,
        "the project's build script",
    ),
    (CARGO_TOML, "target", Safety::Regenerable, "cargo build"),
    (PODFILE, "Pods", Safety::Regenerable, "pod install"),
    (PODFILE, "build", Safety::Regenerable, "xcodebuild"),
    (PACKAGE_SWIFT, ".build", Safety::Regenerable, "swift build"),
    (
        PACKAGE_SWIFT,
        ".swiftpm",
        Safety::Regenerable,
        "swift build / Xcode",
    ),
    (POM_XML, "target", Safety::Regenerable, "mvn package"),
    (GRADLE, "build", Safety::Regenerable, "gradle build"),
    (GRADLE, ".gradle", Safety::Regenerable, "gradle build"),
    (
        PUBSPEC,
        ".dart_tool",
        Safety::Regenerable,
        "flutter pub get",
    ),
    (PUBSPEC, "build", Safety::Regenerable, "flutter build"),
    (
        PYTHON_PROJECT | PY_FILE,
        "__pycache__",
        Safety::Regenerable,
        "running Python",
    ),
    (
        PYTHON_PROJECT | PY_FILE,
        ".pytest_cache",
        Safety::Regenerable,
        "pytest",
    ),
    (
        PYTHON_PROJECT | PY_FILE,
        ".mypy_cache",
        Safety::Regenerable,
        "mypy",
    ),
    (
        PYTHON_PROJECT | PY_FILE,
        ".ruff_cache",
        Safety::Regenerable,
        "ruff",
    ),
    (
        PYTHON_PROJECT,
        ".venv",
        Safety::Review,
        "python -m venv + pip install",
    ),
    (
        PYTHON_PROJECT,
        "venv",
        Safety::Review,
        "python -m venv + pip install",
    ),
    (CMAKE, "build", Safety::Regenerable, "cmake --build"),
    (DOTNET, "bin", Safety::Regenerable, "dotnet build"),
    (DOTNET, "obj", Safety::Regenerable, "dotnet build"),
];

/// Never look for projects inside these (they contain package.json etc. of
/// dependencies, not projects of yours).
pub(crate) const NEVER_DESCEND: &[&str] = &[
    "node_modules",
    ".git",
    "Pods",
    ".venv",
    "venv",
    "site-packages",
];

/// Top-level folders of the home directory that hold app/tool data, not projects.
/// (Hidden top-level folders such as ~/.npm, ~/.nvm are skipped too.)
const SKIP_AT_ROOT: &[&str] = &["Library", "Applications", ".Trash"];

#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub safety: Safety,
    pub regenerate: &'static str,
    /// Inside a git repo: does git ignore this folder? `Some(false)` means it may
    /// hold tracked or not-yet-committed files, so it is never cleaned.
    pub git_ignored: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub path: PathBuf,
    /// Kinds of marker found ("node", "rust", ...).
    pub kinds: Vec<&'static str>,
    pub artifacts: Vec<Artifact>,
    pub artifact_bytes: u64,
    /// Newest mtime of the project's own files, excluding artifacts and .git.
    pub last_active: i64,
    pub git: Option<GitInfo>,
    /// Main repository path: worktrees and nested packages share it.
    pub group: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    pub root: PathBuf,
    pub branch: Option<String>,
    /// None when git isn't available or timed out.
    pub dirty: Option<bool>,
    /// Main repository, when this is a linked worktree.
    pub worktree_of: Option<PathBuf>,
}

fn kinds(m: u32) -> Vec<&'static str> {
    let mut k = Vec::new();
    for (bit, name) in [
        (PACKAGE_JSON, "node"),
        (CARGO_TOML, "rust"),
        (PODFILE, "cocoapods"),
        (PACKAGE_SWIFT, "swift"),
        (POM_XML, "maven"),
        (GRADLE, "gradle"),
        (PUBSPEC, "flutter"),
        (PYTHON_PROJECT | PY_FILE, "python"),
        (CMAKE, "cmake"),
        (DOTNET, "dotnet"),
    ] {
        if m & bit != 0 {
            k.push(name);
        }
    }
    k
}

/// Artifact match for child `name` of a directory with markers `m`.
fn artifact_kind(m: u32, name: &str) -> Option<(Safety, &'static str)> {
    TABLE
        .iter()
        .find(|(bits, n, _, _)| m & bits != 0 && *n == name)
        .map(|(_, _, s, r)| (*s, *r))
}

/// Find projects under `root` (whose tree is `tree`), skipping any path in `excluded`
/// (catalog rule locations).
pub fn detect(tree: &DirNode, root: &Path, excluded: &[PathBuf]) -> Vec<Project> {
    let mut out = Vec::new();
    let mut git_stack: Vec<(PathBuf, u32)> = Vec::new();
    visit(tree, root, true, excluded, &mut git_stack, &mut out);
    out
}

fn visit(
    node: &DirNode,
    path: &Path,
    is_root: bool,
    excluded: &[PathBuf],
    git_stack: &mut Vec<(PathBuf, u32)>,
    out: &mut Vec<Project>,
) {
    let m = node.markers;
    let pushed_git = m & (GIT_DIR | GIT_FILE) != 0;
    if pushed_git {
        git_stack.push((path.to_path_buf(), m));
    }

    let mut artifacts = Vec::new();
    for c in &node.children {
        if let Some((safety, regenerate)) = artifact_kind(m, &c.name) {
            artifacts.push(Artifact {
                path: path.join(&c.name),
                name: c.name.clone(),
                size: c.size,
                safety,
                regenerate,
                git_ignored: None,
            });
        }
    }
    if !artifacts.is_empty() {
        let git = git_stack.last().map(|(root, marks)| GitInfo {
            root: root.clone(),
            branch: None,
            dirty: None,
            worktree_of: if marks & GIT_FILE != 0 {
                worktree_main(root)
            } else {
                None
            },
        });
        let group = git
            .as_ref()
            .map(|g| g.worktree_of.clone().unwrap_or_else(|| g.root.clone()))
            .unwrap_or_else(|| path.to_path_buf());
        out.push(Project {
            path: path.to_path_buf(),
            kinds: kinds(m),
            artifact_bytes: artifacts.iter().map(|a| a.size).sum(),
            artifacts,
            last_active: active_mtime(node),
            git,
            group,
        });
    }

    for c in &node.children {
        let name = c.name.as_str();
        if artifact_kind(m, name).is_some()
            || NEVER_DESCEND.contains(&name)
            || name.ends_with(".app")
            || (is_root && (name.starts_with('.') || SKIP_AT_ROOT.contains(&name)))
        {
            continue;
        }
        let child = path.join(name);
        if excluded.iter().any(|e| child.starts_with(e)) {
            continue;
        }
        visit(c, &child, false, excluded, git_stack, out);
    }
    if pushed_git {
        git_stack.pop();
    }
}

/// Newest file mtime in the project, ignoring artifacts and `.git` at every level.
pub(crate) fn active_mtime(node: &DirNode) -> i64 {
    let mut t = node.files_mtime;
    for c in &node.children {
        let n = c.name.as_str();
        if n == ".git" || NEVER_DESCEND.contains(&n) || artifact_kind(node.markers, n).is_some() {
            continue;
        }
        t = t.max(active_mtime(c));
    }
    t
}

/// For a linked worktree, `.git` is a file: `gitdir: /main/.git/worktrees/<name>`.
pub(crate) fn worktree_main(root: &Path) -> Option<PathBuf> {
    let s = std::fs::read_to_string(root.join(".git")).ok()?;
    let gitdir = s.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let idx = gitdir.find("/.git/worktrees/")?;
    Some(PathBuf::from(&gitdir[..idx]))
}

/// Fill in branch, dirty state and which artifacts git ignores. Two git calls
/// per repository (`status`, `check-ignore`), repositories in parallel.
pub fn fill_git(projects: &mut [Project], git: Option<&Path>) {
    use rayon::prelude::*;
    use std::collections::{HashMap, HashSet};
    let mut by_root: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for p in projects.iter() {
        if let Some(g) = &p.git {
            by_root
                .entry(g.root.clone())
                .or_default()
                .extend(p.artifacts.iter().map(|a| a.path.clone()));
        }
    }
    type Info = (Option<String>, Option<bool>, Option<HashSet<PathBuf>>);
    let info: HashMap<PathBuf, Info> = by_root
        .into_par_iter()
        .map(|(r, arts)| {
            let root_arg = r.display().to_string();
            let timeout = std::time::Duration::from_secs(20);
            let dirty = git.and_then(|g| {
                run(g, &["-C", &root_arg, "status", "--porcelain"], "", timeout)
                    .filter(|(code, _)| *code == 0)
                    .map(|(_, out)| !out.trim().is_empty())
            });
            // check-ignore exits 1 when nothing is ignored; both are answers.
            let input: String = arts.iter().map(|a| format!("{}\n", a.display())).collect();
            let ignored = git.and_then(|g| {
                run(
                    g,
                    &["-C", &root_arg, "check-ignore", "--stdin"],
                    &input,
                    timeout,
                )
                .filter(|(code, _)| *code == 0 || *code == 1)
                .map(|(_, out)| {
                    out.lines()
                        .map(|l| PathBuf::from(l.trim_end_matches('/')))
                        .collect()
                })
            });
            (r.clone(), (head_branch(&r), dirty, ignored))
        })
        .collect();
    for p in projects {
        if let Some(g) = &mut p.git {
            if let Some((b, d, ign)) = info.get(&g.root) {
                g.branch = b.clone();
                g.dirty = *d;
                for a in &mut p.artifacts {
                    a.git_ignored = ign.as_ref().map(|set| set.contains(&a.path));
                }
            }
        }
    }
}

/// Run a command with `input` on stdin; returns (exit code, stdout).
pub(crate) fn run(
    bin: &Path,
    args: &[&str],
    input: &str,
    timeout: std::time::Duration,
) -> Option<(i32, String)> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let mut child = Command::new(bin)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0") // don't let `git status` write the index
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let input = input.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut out_pipe = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out_pipe.read_to_string(&mut s);
        s
    });
    let start = std::time::Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(-1),
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let _ = writer.join();
    Some((code, reader.join().ok()?))
}

pub(crate) fn head_branch(root: &Path) -> Option<String> {
    let dotgit = root.join(".git");
    let gitdir = if dotgit.is_dir() {
        dotgit
    } else {
        let s = std::fs::read_to_string(&dotgit).ok()?;
        // gitdir may be relative to the worktree.
        root.join(s.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim())
    };
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    match head.trim().strip_prefix("ref: refs/heads/") {
        Some(b) => Some(b.to_string()),
        None => Some(format!(
            "detached {}",
            &head.trim()[..head.trim().len().min(8)]
        )),
    }
}
