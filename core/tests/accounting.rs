//! Fixture tests for size accounting (PLAN.md §7). macOS / APFS only.
//! Every test checks both backends and compares against `du -skx`.

use spacesense_core::{scan, Backend, ScanOptions, ScanResult};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const BACKENDS: [Backend; 2] = [Backend::Bulk, Backend::Stat];

fn ss(root: &Path, backend: Backend) -> ScanResult {
    scan(
        root,
        &ScanOptions {
            backend,
            threads: 4,
            ..Default::default()
        },
    )
    .expect("scan")
}

fn du_kib(root: &Path) -> u64 {
    let out = Command::new("du")
        .arg("-skx")
        .arg(root)
        .output()
        .expect("du");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Non-zero, non-repeating-ish content so APFS can't do anything clever.
    let data: Vec<u8> = (0..bytes).map(|i| (i * 31 % 251) as u8).collect();
    fs::write(path, data).unwrap();
}

fn sh(cmd: &str, dir: &Path) {
    let st = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(st.success(), "{cmd} failed");
}

fn assert_matches_du(root: &Path) -> [ScanResult; 2] {
    let du = du_kib(root);
    let rs = BACKENDS.map(|b| ss(root, b));
    for r in &rs {
        assert_eq!(r.total_bytes / 1024, du, "{:?} total vs du -skx", r.backend);
    }
    assert_eq!(rs[0].total_bytes, rs[1].total_bytes, "backends disagree");
    assert_eq!(rs[0].stats.files, rs[1].stats.files);
    assert_eq!(rs[0].stats.dirs, rs[1].stats.dirs);
    rs
}

#[test]
fn plain_tree() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..50 {
        write(&t.path().join(format!("a/b{}/f{i}", i % 7)), 1000 * i + 1);
    }
    write(&t.path().join("big"), 3_000_000);
    let [r, _] = assert_matches_du(t.path());
    assert_eq!(r.stats.files, 51);
    assert_eq!(r.stats.dirs, 1 + 1 + 7); // root, a, b0..b6
}

#[test]
fn hard_links_counted_once() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("orig"), 2_000_000);
    fs::create_dir(t.path().join("sub")).unwrap();
    fs::hard_link(t.path().join("orig"), t.path().join("link1")).unwrap();
    fs::hard_link(t.path().join("orig"), t.path().join("sub/link2")).unwrap();
    let rs = assert_matches_du(t.path());
    for r in &rs {
        assert_eq!(r.stats.hardlinks_deduped, 2);
        assert!(r.total_bytes < 2 * 2_000_000, "hard link counted twice");
    }
}

#[test]
fn sparse_file_uses_allocated_size() {
    let t = tempfile::tempdir().unwrap();
    sh("mkfile -n 200m sparse", t.path());
    assert_eq!(
        fs::metadata(t.path().join("sparse")).unwrap().len(),
        200 * 1024 * 1024
    );
    let rs = assert_matches_du(t.path());
    for r in &rs {
        assert!(
            r.total_bytes < 1024 * 1024,
            "sparse file counted at logical size"
        );
    }
}

#[test]
fn apfs_clone_is_plain_sum() {
    // v1 policy: clones are summed like du does (they overstate real usage; footnote).
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("orig"), 1_000_000);
    sh("cp -c orig clone", t.path());
    let rs = assert_matches_du(t.path());
    assert!(rs[0].total_bytes >= 2 * 1_000_000);
}

#[test]
fn symlinks_not_followed() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("d/f"), 500_000);
    std::os::unix::fs::symlink(".", t.path().join("d/loop")).unwrap();
    std::os::unix::fs::symlink("..", t.path().join("d/up")).unwrap();
    std::os::unix::fs::symlink("/usr", t.path().join("d/outside")).unwrap();
    let [r, _] = assert_matches_du(t.path());
    assert_eq!(r.stats.dirs, 2);
    assert_eq!(r.stats.files, 4); // f + 3 symlinks
}

#[test]
fn unreadable_dir_is_reported_not_fatal() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("ok/f"), 100_000);
    write(&t.path().join("locked/hidden"), 100_000);
    let locked = t.path().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let rs = BACKENDS.map(|b| ss(t.path(), b));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    for r in &rs {
        assert_eq!(r.stats.dir_errors, 1);
        let node = r.tree.children.iter().find(|c| c.name == "locked").unwrap();
        assert!(node.error.is_some());
        assert_eq!(r.stats.files, 1);
    }
}

#[test]
fn deep_nesting() {
    let t = tempfile::tempdir().unwrap();
    let mut p = t.path().to_path_buf();
    for i in 0..400 {
        p.push(format!("d{}", i % 10));
    }
    // PATH_MAX is 1024; stay under it with short names.
    let p: std::path::PathBuf = p.iter().take(t.path().iter().count() + 300).collect();
    write(&p.join("leaf"), 10_000);
    let [r, _] = assert_matches_du(t.path());
    assert_eq!(r.stats.files, 1);
}

#[test]
fn many_entries_span_multiple_bulk_batches() {
    let t = tempfile::tempdir().unwrap();
    // Long names so 256 KiB buffer needs several getattrlistbulk calls.
    for i in 0..5000 {
        let name = format!("{i:05}-{}", "x".repeat(200));
        fs::write(t.path().join(name), b"hi").unwrap();
    }
    let [r, _] = assert_matches_du(t.path());
    assert_eq!(r.stats.files, 5000);
}

#[test]
fn newest_mtime_propagates() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("old/a"), 10);
    write(&t.path().join("new/b"), 10);
    sh(
        "touch -t 202001010000 old/a old new/b; touch -t 202501010000 new/b",
        t.path(),
    );
    for b in BACKENDS {
        let r = ss(t.path(), b);
        let get = |n: &str| {
            r.tree
                .children
                .iter()
                .find(|c| c.name == n)
                .unwrap()
                .newest_mtime
        };
        assert!(get("new") > get("old"), "{b:?}");
        assert_eq!(r.tree.newest_mtime.max(get("new")), r.tree.newest_mtime);
    }
}

#[test]
fn skipped_folders_are_not_entered() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("keep/f"), 100_000);
    write(&t.path().join("Library/Mobile Documents/big"), 3_000_000);
    write(&t.path().join("Library/other/f"), 50_000);
    for backend in BACKENDS {
        let opts = ScanOptions {
            backend,
            threads: 4,
            skip: vec![t.path().join("Library/Mobile Documents")],
            ..Default::default()
        };
        let r = scan(t.path(), &opts).unwrap();
        let lib = r.tree.children.iter().find(|c| c.name == "Library").unwrap();
        let cloud = lib.children.iter().find(|c| c.name == "Mobile Documents").unwrap();
        assert_eq!(cloud.error.as_deref(), Some(spacesense_core::SKIPPED));
        assert!(cloud.children.is_empty() && cloud.files == 0);
        assert!(r.total_bytes < 1_000_000, "{backend:?}: skipped content was counted");
        assert_eq!(r.stats.files, 2);
    }
}
