//! Project detector + dry-run planner on a fixture tree (PLAN.md §7).
//! Nothing here deletes anything: `plan` only describes actions.

use spacesense_core::plan::{self, Selection, Step};
use spacesense_core::report::{self, Report, ReportOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn w(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

fn sh(dir: &Path, cmd: &str) {
    let ok = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "{cmd}");
}

/// Set every file and dir under `p` to 2020 (stale).
fn age(p: &Path) {
    sh(p, "find . -exec touch -h -t 202001010000 {} +");
}

struct Fixture {
    _t: tempfile::TempDir,
    home: PathBuf,
}

fn fixture() -> Fixture {
    let t = tempfile::tempdir().unwrap();
    // Three levels deep so artifact paths pass the "too broad" check.
    let home = t
        .path()
        .join("Users/me")
        .canonicalize()
        .unwrap_or_else(|_| {
            fs::create_dir_all(t.path().join("Users/me")).unwrap();
            t.path().join("Users/me").canonicalize().unwrap()
        });
    let h = &home;

    // Stale node project with nested deps and a Next build.
    w(&h.join("old-node/package.json"), "{}");
    w(&h.join("old-node/src/index.js"), "x");
    w(&h.join("old-node/node_modules/dep/package.json"), "{}");
    w(
        &h.join("old-node/node_modules/dep/node_modules/inner/package.json"),
        "{}",
    );
    w(&h.join("old-node/.next/cache/blob"), &"x".repeat(5000));
    w(&h.join("old-node/dist/app.js"), "x");
    age(&h.join("old-node"));
    // Finder touching .DS_Store must not make it "active".
    w(&h.join("old-node/.DS_Store"), "finder");

    // Stale Rust project.
    w(&h.join("rusty/Cargo.toml"), "[package]");
    w(&h.join("rusty/target/debug/bin"), &"x".repeat(9000));
    age(&h.join("rusty"));

    // Active node project (just written).
    w(&h.join("active/package.json"), "{}");
    w(&h.join("active/node_modules/dep/package.json"), "{}");

    // Not projects of the user: hidden top-level and Library.
    w(&h.join(".tool/package.json"), "{}");
    w(&h.join(".tool/node_modules/x/package.json"), "{}");
    w(&h.join("Library/App/package.json"), "{}");
    w(&h.join("Library/App/node_modules/x/package.json"), "{}");

    // Git repo with node_modules ignored, a tracked gradle build/ dir, and a worktree.
    let repo = h.join("repo");
    w(&repo.join("package.json"), "{}");
    w(&repo.join(".gitignore"), "node_modules/\n");
    w(&repo.join("node_modules/dep/package.json"), "{}");
    w(&repo.join("android/build.gradle"), "x");
    w(
        &repo.join("android/build/tracked.txt"),
        "committed on purpose",
    );
    sh(&repo, "git init -q -b main && git add .gitignore package.json android && git -c user.name=t -c user.email=t@t commit -qm init");
    sh(&repo, "git worktree add -q ../wt -b feature");
    w(&h.join("wt/node_modules/dep/package.json"), "{}");
    w(&repo.join("uncommitted.txt"), "work in progress");
    age(&repo);
    age(&h.join("wt"));

    Fixture { _t: t, home }
}

fn build(f: &Fixture) -> Report {
    let empty = tempfile::tempdir().unwrap();
    let opts = ReportOptions {
        catalog_dir: Some(empty.path().to_path_buf()),
        home: Some(f.home.clone()),
        ..Default::default()
    };
    report::build(&f.home, &opts).unwrap()
}

fn project<'a>(
    r: &'a Report,
    rel: &str,
    home: &Path,
) -> Option<&'a spacesense_core::projects::Project> {
    r.projects.iter().find(|p| p.path == home.join(rel))
}

#[test]
fn detects_projects_and_skips_tool_data() {
    let f = fixture();
    let r = build(&f);
    let h = &f.home;
    let names: Vec<String> = r
        .projects
        .iter()
        .map(|p| p.path.strip_prefix(h).unwrap().display().to_string())
        .collect();

    let old = project(&r, "old-node", h).expect("old-node");
    let mut arts: Vec<&str> = old.artifacts.iter().map(|a| a.name.as_str()).collect();
    arts.sort();
    assert_eq!(arts, [".next", "dist", "node_modules"]);
    assert!(project(&r, "rusty", h).is_some());
    assert!(project(&r, "active", h).is_some());
    // Dependencies inside node_modules are not projects.
    assert!(
        !names.iter().any(|n| n.contains("node_modules")),
        "{names:?}"
    );
    // Hidden top-level folders and ~/Library are tool/app data.
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with(".tool") || n.starts_with("Library")),
        "{names:?}"
    );

    // Sizes come from the scan tree and must match du.
    let du = Command::new("du")
        .args(["-skx"])
        .arg(h.join("rusty/target"))
        .output()
        .unwrap();
    let du_kib: u64 = String::from_utf8_lossy(&du.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        project(&r, "rusty", h).unwrap().artifact_bytes / 1024,
        du_kib
    );

    // .DS_Store doesn't count as activity.
    assert!(
        r.generated_at - old.last_active > 365 * 86_400,
        "old-node looks active"
    );
    assert!(r.generated_at - project(&r, "active", h).unwrap().last_active < 3600);
}

#[test]
fn git_state_worktrees_and_ignores() {
    let f = fixture();
    let r = build(&f);
    let h = &f.home;
    let repo = project(&r, "repo", h).expect("repo");
    let g = repo.git.as_ref().unwrap();
    assert_eq!(g.branch.as_deref(), Some("main"));
    assert_eq!(g.dirty, Some(true));
    assert_eq!(repo.artifacts[0].git_ignored, Some(true));

    let wt = project(&r, "wt", h).expect("worktree");
    let wg = wt.git.as_ref().unwrap();
    assert_eq!(wg.worktree_of.as_deref(), Some(h.join("repo").as_path()));
    assert_eq!(wg.branch.as_deref(), Some("feature"));
    assert_eq!(wt.group, h.join("repo"));

    let android = project(&r, "repo/android", h).expect("gradle project");
    let build = android
        .artifacts
        .iter()
        .find(|a| a.name == "build")
        .unwrap();
    assert_eq!(
        build.git_ignored,
        Some(false),
        "tracked build/ must be flagged"
    );
}

#[test]
fn default_plan_respects_guards() {
    let f = fixture();
    let r = build(&f);
    let h = &f.home;
    let p = plan::build(&r, &Selection::default());
    let deleted: Vec<PathBuf> = p
        .actions
        .iter()
        .flat_map(|a| &a.steps)
        .filter_map(|s| match s {
            Step::DeleteDir { path } => Some(path.clone()),
            Step::Trash { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let has = |rel: &str| deleted.contains(&h.join(rel));

    assert!(has("old-node/node_modules") && has("old-node/.next") && has("rusty/target"));
    assert!(
        has("repo/node_modules"),
        "dirty repo's ignored node_modules is still fine"
    );
    assert!(has("wt/node_modules"));
    assert!(
        !has("old-node/dist"),
        "dist/ is review-only, not in the default plan"
    );
    assert!(
        !has("active/node_modules"),
        "active project must be protected"
    );
    assert!(
        !has("repo/android/build"),
        "tracked build/ must never be planned"
    );
    for d in &deleted {
        assert!(d.starts_with(h), "{d:?} outside home");
        assert!(!d.starts_with(h.join("Library")) && !d.starts_with(h.join(".tool")));
    }
    assert!(p
        .skipped
        .iter()
        .any(|s| s.target.ends_with("active") && s.reason.contains("active")));
    assert!(p
        .skipped
        .iter()
        .any(|s| s.target.ends_with("android/build") && s.reason.contains("git")));

    // Naming the active project explicitly still needs --include-active.
    let named = plan::build(
        &r,
        &Selection {
            targets: vec![h.join("active").display().to_string()],
            include_active: false,
        },
    );
    assert!(named.actions.is_empty());
    let forced = plan::build(
        &r,
        &Selection {
            targets: vec![h.join("active").display().to_string()],
            include_active: true,
        },
    );
    assert_eq!(forced.actions.len(), 1);

    // Unknown targets are reported, not ignored.
    let bogus = plan::build(
        &r,
        &Selection {
            targets: vec!["/".into()],
            include_active: true,
        },
    );
    assert!(bogus.actions.is_empty());
    assert!(bogus.skipped.iter().any(|s| s.target == "/"));
}
