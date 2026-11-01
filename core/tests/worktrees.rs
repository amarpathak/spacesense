//! Worktree scanner on a fixture repo with one worktree per status.
//! Read-only: nothing here removes a worktree.

use spacesense_core::report::{self, ReportOptions};
use spacesense_core::worktrees::{Status, Worktree};
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
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap()
        .success();
    assert!(ok, "{cmd}");
}

fn age(p: &Path) {
    sh(p, "find . -exec touch -h -t 202001010000 {} +");
}

fn find<'a>(ws: &'a [Worktree], p: &Path) -> &'a Worktree {
    ws.iter()
        .find(|w| w.path == p)
        .unwrap_or_else(|| panic!("no worktree {}", p.display()))
}

#[test]
fn classifies_worktrees() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("Users/me")).unwrap();
    let home: PathBuf = t.path().join("Users/me").canonicalize().unwrap();
    let repo = home.join("code/app");
    w(&repo.join("package.json"), "{}");
    w(&repo.join(".gitignore"), "node_modules/\n");
    sh(&repo, "git init -q -b main && git add . && git commit -qm init");

    // done: branch merged into main, idle, with a big node_modules.
    sh(&repo, "git worktree add -q ../app-done -b done");
    w(&home.join("code/app-done/node_modules/x/blob"), &"x".repeat(50_000));
    // idle: unmerged commit, clean, idle.
    sh(&repo, "git worktree add -q ../app-idle -b idle");
    w(&home.join("code/app-idle/f.txt"), "x");
    sh(&home.join("code/app-idle"), "git add f.txt && git commit -qm wip");
    // dirty: uncommitted file.
    sh(&repo, "git worktree add -q ../app-dirty -b dirty");
    w(&home.join("code/app-dirty/notes.txt"), "unsaved");
    // locked.
    sh(&repo, "git worktree add -q ../app-locked -b locked && git worktree lock ../app-locked");
    // prunable: folder deleted by hand.
    sh(&repo, "git worktree add -q ../app-gone -b gone && rm -rf ../app-gone");
    // active: recently touched, made by an agent tool inside the repo.
    sh(&repo, "git worktree add -q .claude/worktrees/agent-1 -b agent-1");

    for d in ["code/app-done", "code/app-idle", "code/app-dirty", "code/app-locked"] {
        age(&home.join(d));
    }

    let empty = tempfile::tempdir().unwrap();
    let opts = ReportOptions {
        catalog_dir: Some(empty.path().to_path_buf()),
        home: Some(home.clone()),
        ..Default::default()
    };
    let r = report::build(&home, &opts).unwrap();
    let ws = &r.worktrees;
    assert_eq!(ws.len(), 6, "{ws:#?}");

    let done = find(ws, &home.join("code/app-done"));
    assert_eq!(done.status, Status::Done, "{done:#?}");
    assert_eq!(done.merged, Some(true));
    assert_eq!(done.base.as_deref(), Some("main"));
    assert!(done.artifact_bytes >= 50_000);
    assert!(done.size.unwrap() >= done.artifact_bytes);
    assert_eq!(done.suggested.as_ref().unwrap()[3..5], ["worktree", "remove"]);

    let idle = find(ws, &home.join("code/app-idle"));
    assert_eq!(idle.status, Status::Idle, "{idle:#?}");
    assert_eq!(idle.merged, Some(false));

    let dirty = find(ws, &home.join("code/app-dirty"));
    assert_eq!(dirty.status, Status::Dirty);
    assert!(dirty.suggested.is_none(), "never suggest removing uncommitted work");

    assert_eq!(find(ws, &home.join("code/app-locked")).status, Status::Locked);

    let gone = find(ws, &home.join("code/app-gone"));
    assert_eq!(gone.status, Status::Prunable);
    assert!(!gone.exists);
    assert_eq!(gone.suggested.as_ref().unwrap()[3..], ["worktree", "prune"]);

    let agent = find(ws, &repo.join(".claude/worktrees/agent-1"));
    assert_eq!(agent.status, Status::Active);
    assert_eq!(agent.created_by, Some("Claude Code"));
    assert!(ws.iter().all(|w| w.repo == repo));
}
