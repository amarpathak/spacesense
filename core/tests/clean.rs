//! Cleanup execution on fixture trees only (PLAN.md §7): nothing here touches
//! real caches. Each test builds a fake home in a temp dir, a catalog pointing
//! into it, and checks what is (and is not) removed.

use spacesense_core::clean::{self, CleanOptions};
use spacesense_core::plan::{self, Action, Plan, Selection, Step};
use spacesense_core::catalog::Safety;
use spacesense_core::report::{self, Report, ReportOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn w(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

fn sh(dir: &Path, cmd: &str) {
    assert!(Command::new("sh").arg("-c").arg(cmd).current_dir(dir).status().unwrap().success(), "{cmd}");
}

struct Fx {
    _t: tempfile::TempDir,
    home: PathBuf,
    catalog: PathBuf,
    log: PathBuf,
}

/// Fake home three levels deep (paths must have ≥ 3 components to be cleanable).
fn fixture() -> Fx {
    let t = tempfile::tempdir().unwrap();
    let base = t.path().canonicalize().unwrap();
    let home = base.join("Users/me");
    fs::create_dir_all(&home).unwrap();
    let catalog = base.join("catalog");
    fs::create_dir_all(&catalog).unwrap();
    let log = base.join("ops.log");
    Fx { _t: t, home, catalog, log }
}

fn rule(fx: &Fx, id: &str, body: &str) {
    let src = format!(
        "id = \"{id}\"\nname = \"{id}\"\ncategory = \"package-cache\"\ncomes_back = \"x\"\nexplanation = \"x\"\nverified = \"test\"\n{body}"
    );
    fs::write(fx.catalog.join(format!("{id}.toml")), src).unwrap();
}

fn report(fx: &Fx) -> Report {
    report::build(
        &fx.home,
        &ReportOptions {
            catalog_dir: Some(fx.catalog.clone()),
            home: Some(fx.home.clone()),
            ..Default::default()
        },
    )
    .unwrap()
}

fn opts(fx: &Fx, r: &Report) -> CleanOptions {
    let mut o = CleanOptions::new(&fx.home, plan::protected_paths(r));
    o.log_path = fx.log.clone();
    o
}

fn select(r: &Report, targets: &[&str]) -> Plan {
    plan::build(r, &Selection { targets: targets.iter().map(|s| s.to_string()).collect(), ..Default::default() })
}

#[test]
fn delete_contents_keeps_folder_and_logs() {
    let fx = fixture();
    let h = &fx.home;
    w(&h.join(".fakecache/a/b"), 200_000);
    w(&h.join(".fakecache/c"), 100_000);
    w(&h.join("keep/me"), 50_000);
    rule(&fx, "fake-cache", "paths = [\"~/.fakecache\"]\nsafety = \"cache\"\nfallback = \"delete-contents\"\n");
    let r = report(&fx);
    let p = select(&r, &["fake-cache"]);
    assert_eq!(p.actions.len(), 1, "{:?}", p.skipped);
    let res = clean::execute(&p, &opts(&fx, &r), &|_, _, _| {});
    assert!(res.actions[0].steps[0].ok, "{:?}", res.actions[0].steps[0]);
    assert!(h.join(".fakecache").is_dir(), "folder itself must stay");
    assert_eq!(fs::read_dir(h.join(".fakecache")).unwrap().count(), 0);
    assert!(h.join("keep/me").exists());
    assert!(res.freed_bytes >= 300_000);
    let log = fs::read_to_string(&fx.log).unwrap();
    assert!(log.contains("\"target\":\"fake-cache\"") && log.contains("\"ok\":true"), "{log}");
}

#[test]
fn official_command_runs_and_is_measured() {
    let fx = fixture();
    let h = &fx.home;
    w(&h.join(".toolcache/blob"), 400_000);
    // A fake tool whose "cache clean" empties its cache.
    let tool = fx.catalog.join("faketool");
    fs::write(&tool, format!("#!/bin/sh\nrm -rf '{}'/*\n", h.join(".toolcache").display())).unwrap();
    sh(&fx.catalog, "chmod +x faketool");
    rule(
        &fx,
        "tool-cache",
        &format!(
            "paths = [\"~/.toolcache\"]\nsafety = \"cache\"\nfallback = \"none\"\ntool = \"faketool-not-on-path\"\ntool_paths = [\"{}\"]\nofficial_cmd = [\"faketool\", \"cache\", \"clean\"]\n",
            tool.display()
        ),
    );
    let r = report(&fx);
    let p = select(&r, &["tool-cache"]);
    assert!(matches!(p.actions[0].steps[0], Step::Run { .. }));
    let res = clean::execute(&p, &opts(&fx, &r), &|_, _, _| {});
    let s = &res.actions[0].steps[0];
    assert!(s.ok && s.exit_code == Some(0), "{s:?}");
    assert!(s.freed_bytes >= 400_000, "{s:?}");
}

#[test]
fn projects_guards_at_execution_time() {
    let fx = fixture();
    let h = &fx.home;
    // Stale project in git with node_modules ignored.
    let p1 = h.join("old");
    w(&p1.join("package.json"), 2);
    w(&p1.join(".gitignore"), 0);
    fs::write(p1.join(".gitignore"), "node_modules/\n").unwrap();
    w(&p1.join("node_modules/dep/index.js"), 300_000);
    w(&p1.join("src/a.js"), 10);
    sh(&p1, "git init -q && git add .gitignore package.json src && git -c user.name=t -c user.email=t@t commit -qm i");
    sh(&p1, "find . -path ./.git -prune -o -exec touch -h -t 202001010000 {} +");
    // Active project.
    w(&h.join("active/package.json"), 2);
    w(&h.join("active/node_modules/x/i.js"), 100_000);
    // Stale project whose node_modules will become tracked after planning.
    let p3 = h.join("sneaky");
    w(&p3.join("package.json"), 2);
    fs::write(p3.join(".gitignore"), "node_modules/\n").unwrap();
    w(&p3.join("node_modules/lib/index.js"), 100_000);
    sh(&p3, "git init -q && git add .gitignore package.json && git -c user.name=t -c user.email=t@t commit -qm i");
    sh(&p3, "find . -path ./.git -prune -o -exec touch -h -t 202001010000 {} +");

    let r = report(&fx);
    // Active project can't even be planned without include_active.
    assert!(select(&r, &[h.join("active").to_str().unwrap()]).actions.is_empty());

    let p = select(&r, &[p1.to_str().unwrap(), p3.join("node_modules").to_str().unwrap()]);
    assert_eq!(p.actions.len(), 2, "{:?}", p.skipped);
    // After the plan was made, someone starts tracking sneaky/node_modules.
    fs::write(p3.join(".gitignore"), "").unwrap();
    sh(&p3, "git add -f node_modules .gitignore && git -c user.name=t -c user.email=t@t commit -qm track");

    let res = clean::execute(&p, &opts(&fx, &r), &|_, _, _| {});
    assert!(!p1.join("node_modules").exists(), "stale ignored node_modules should be gone");
    assert!(p1.join("src/a.js").exists() && p1.join("package.json").exists(), "source must stay");
    assert!(p3.join("node_modules/lib/index.js").exists(), "now-tracked folder must survive");
    let sneaky = res.actions.iter().find(|a| a.target.contains("sneaky")).unwrap();
    assert!(sneaky.steps[0].skipped && sneaky.steps[0].message.contains("git"), "{:?}", sneaky.steps[0]);
    assert!(h.join("active/node_modules/x/i.js").exists());
}

#[test]
fn refuses_symlink_swap_outside_home_and_running_apps() {
    let fx = fixture();
    let h = &fx.home;
    let outside = fx.catalog.join("precious");
    w(&outside.join("keep.txt"), 1000);
    w(&h.join("proj/build/out"), 1000);

    let r = report(&fx);
    let o = opts(&fx, &r);
    // Planned as a real folder, then swapped for a symlink pointing outside home.
    fs::remove_dir_all(h.join("proj/build")).unwrap();
    std::os::unix::fs::symlink(&outside, h.join("proj/build")).unwrap();
    let mk = |steps: Vec<Step>, quit: Vec<String>| Plan {
        actions: vec![Action {
            target: "t".into(),
            source: "test".into(),
            title: "t".into(),
            safety: Safety::Cache,
            bytes: 0,
            steps,
            warnings: vec![],
            blockers: vec![],
            quit_apps: quit,
            suggested: false,
        }],
        skipped: vec![],
        total_bytes: 0,
    };
    let res = clean::execute(&mk(vec![Step::DeleteDir { path: h.join("proj/build") }, Step::DeleteContents { path: h.join("proj/build") }], vec![]), &o, &|_, _, _| {});
    assert!(res.actions[0].steps.iter().all(|s| s.skipped), "{:?}", res.actions[0].steps);
    assert!(outside.join("keep.txt").exists(), "symlink target outside home must survive");

    // Directly outside home, home itself, and too-broad paths.
    for p in [outside.clone(), h.clone(), PathBuf::from("/Users")] {
        let res = clean::execute(&mk(vec![Step::DeleteDir { path: p.clone() }], vec![]), &o, &|_, _, _| {});
        assert!(res.actions[0].steps[0].skipped, "{p:?} was not refused");
    }
    assert!(outside.join("keep.txt").exists() && h.exists());

    // An app that must be quit is running (launchd always is): nothing happens.
    w(&h.join("cache2/x"), 1000);
    let res = clean::execute(&mk(vec![Step::DeleteContents { path: h.join("cache2") }], vec!["launchd".into()]), &o, &|_, _, _| {});
    assert!(res.actions[0].steps[0].skipped && res.actions[0].steps[0].message.contains("launchd"));
    assert!(h.join("cache2/x").exists());
}

#[test]
fn never_auto_and_unknown_targets_are_refused() {
    let fx = fixture();
    let h = &fx.home;
    w(&h.join("Backups/dev/data"), 1000);
    rule(&fx, "backups", "paths = [\"~/Backups\"]\nsafety = \"never_auto\"\nfallback = \"none\"\n");
    let r = report(&fx);
    let p = select(&r, &["backups", "/", h.to_str().unwrap()]);
    assert!(p.actions.is_empty());
    assert_eq!(p.skipped.len(), 3, "{:?}", p.skipped);
    // Candidates list (what the app shows) never offers never-auto items.
    let c = plan::build(&r, &Selection { candidates: true, ..Default::default() });
    assert!(c.actions.iter().all(|a| a.target != "backups"));
}

/// Moves a fixture file to the real Trash (harmless, but it does land in the
/// user's Trash), so it only runs on request: cargo test -- --ignored
#[test]
#[ignore]
fn trash_moves_file_out() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("spacesense-trash-test.txt");
    fs::write(&f, "test file from SpaceSense's test suite; safe to delete").unwrap();
    spacesense_core::trash::trash(&f).unwrap();
    assert!(!f.exists());
}
