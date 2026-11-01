//! System Data breakdown: parts are exclusive (nested parts subtracted).

use spacesense_core::report::{self, ReportOptions};
use std::fs;
use std::path::Path;

fn w(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

#[test]
fn parts_are_exclusive_and_named() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("Users/me")).unwrap();
    let h = t.path().join("Users/me").canonicalize().unwrap();
    w(&h.join("Library/Caches/app/blob"), 200_000);
    w(&h.join("Library/Application Support/App/db"), 300_000);
    w(&h.join("Library/Application Support/MobileSync/Backup/x"), 400_000);
    w(&h.join("Library/Preferences/p.plist"), 50_000);
    w(&h.join("Library/Mail/V10/m"), 500_000);
    w(&h.join(".npm/_cacache/x"), 600_000);
    w(&h.join(".Trash/old"), 700_000);
    w(&h.join("Documents/doc"), 800_000);

    let empty = tempfile::tempdir().unwrap();
    let opts = ReportOptions {
        catalog_dir: Some(empty.path().to_path_buf()),
        home: Some(h.clone()),
        git: false,
        ..Default::default()
    };
    let r = report::build(&h, &opts).unwrap();
    let part = |id: &str| r.system_data.iter().find(|p| p.id == id);
    let size = |id: &str| part(id).and_then(|p| p.size).unwrap_or(0);
    let near = |a: u64, b: u64| a >= b && a < b + 64 * 1024;

    assert!(near(size("app-caches"), 200_000), "{}", size("app-caches"));
    // MobileSync is its own part, not counted in App data too.
    assert!(near(size("app-support"), 300_000), "{}", size("app-support"));
    assert!(near(size("ios-backups"), 400_000));
    // Other ~/Library: Preferences, but not Mail (own Storage category).
    assert!(near(size("other-library"), 50_000), "{}", size("other-library"));
    // Hidden tool folders, but not the Trash.
    let hidden = part("hidden-tools").unwrap();
    assert!(near(hidden.size.unwrap(), 600_000));
    assert_eq!(hidden.paths, vec![h.join(".npm")]);
    assert_eq!(hidden.top[0].path, h.join(".npm"));
    // Empty parts are dropped; App data lists its biggest folder.
    assert!(part("xcode").is_none());
    let support = part("app-support").unwrap();
    assert_eq!(support.top[0].path, h.join("Library/Application Support/App"));
    assert!(support.top.iter().all(|t| !t.path.ends_with("MobileSync")));
}
