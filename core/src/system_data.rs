//! What's inside macOS's grey "System Data" bar, as named, measured parts.
//!
//! Apple doesn't publish how it computes System Data: roughly everything not
//! in another Storage category (Apps, Documents, Photos, Mail, iCloud Drive,
//! macOS). On a developer's Mac that is mostly ~/Library, hidden tool folders
//! in the home folder, per-user temp files, swap and macOS support volumes.
//! Each part's size is exclusive: nested parts are subtracted, so parts add up.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct SystemPart {
    pub id: &'static str,
    pub name: &'static str,
    pub explanation: &'static str,
    /// None when it can't be measured (e.g. local snapshots).
    pub size: Option<u64>,
    pub paths: Vec<PathBuf>,
    /// macOS creates and removes it by itself; nothing for you to clean.
    pub managed_by_macos: bool,
    /// App screen that lists what's cleanable in it ("developer", "system",
    /// "folders"), if any.
    pub screen: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Biggest folders inside it (filled in by the report from the scan tree).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<TopItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopItem {
    pub path: PathBuf,
    pub size: u64,
}

/// Measures a path: allocated bytes (0 if missing or unreadable).
pub type SizeFn<'a> = dyn FnMut(&Path) -> u64 + 'a;

/// macOS's per-user folder in /var/folders (temp files, per-app caches).
fn darwin_user_dir() -> Option<PathBuf> {
    let mut buf = vec![0u8; 1024];
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_CACHE_DIR,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
        )
    };
    if n == 0 || n > buf.len() {
        return None;
    }
    let s = std::ffi::CStr::from_bytes_until_nul(&buf).ok()?.to_str().ok()?;
    // .../xx/yyyy/C/ → .../xx/yyyy (holds C, T, 0, X).
    let p = Path::new(s.trim_end_matches('/')).parent()?;
    // /var is a symlink to /private/var; use the real path.
    let p = match p.strip_prefix("/var") {
        Ok(rest) => Path::new("/private/var").join(rest),
        Err(_) => p.to_path_buf(),
    };
    p.starts_with("/private/var/folders").then_some(p)
}

/// `hidden_home_dirs`: (path, size) of hidden folders directly in the home
/// folder, from the scan (None when the scan wasn't of the home folder).
pub fn build(
    home: &Path,
    hidden_home_dirs: Option<Vec<(PathBuf, u64)>>,
    size: &mut SizeFn,
    volume_used: &dyn Fn(&Path) -> Option<u64>,
    tm_snapshots: Option<usize>,
) -> Vec<SystemPart> {
    let l = home.join("Library");
    let lib = |s: &str| l.join(s);
    let mut parts = Vec::new();
    let mut part = |id, name, explanation, sz: Option<u64>, paths: Vec<PathBuf>, managed, screen| {
        parts.push(SystemPart {
            id,
            name,
            explanation,
            size: sz,
            paths,
            managed_by_macos: managed,
            screen,
            note: None,
            top: Vec::new(),
        })
    };

    let caches = size(&lib("Caches"));
    let support = size(&lib("Application Support"));
    let mobilesync = size(&lib("Application Support/MobileSync"));
    let containers = size(&lib("Containers")) + size(&lib("Group Containers"));
    let developer = size(&lib("Developer"));
    let logs = size(&lib("Logs"));
    // Mail and Messages have their own Storage categories.
    let own_category = size(&lib("Mail")) + size(&lib("Messages"));
    let library = size(&l);

    part("app-caches", "App caches",
        "Caches apps keep to start faster. Apps rebuild them, so most are safe to clear.",
        Some(caches), vec![lib("Caches")], false, Some("system"));
    part("app-support", "App data",
        "Settings, databases, downloaded models and offline content apps keep. Some is safe to remove (old AI models, leftovers of uninstalled apps), some is your data.",
        Some(support.saturating_sub(mobilesync)), vec![lib("Application Support")], false, Some("folders"));
    part("containers", "Sandboxed apps & Docker",
        "Data of App Store and sandboxed apps. Docker Desktop's disk image lives here too.",
        Some(containers), vec![lib("Containers"), lib("Group Containers")], false, Some("folders"));
    part("xcode", "Xcode & simulators",
        "DerivedData, archives, device support and simulator runtimes. Mostly rebuilt or re-downloaded by Xcode.",
        Some(developer), vec![lib("Developer")], false, Some("developer"));
    part("ios-backups", "iPhone & iPad backups",
        "Local device backups made by Finder. Delete old ones from Finder's device window.",
        Some(mobilesync), vec![lib("Application Support/MobileSync")], false, Some("system"));
    part("logs", "Logs",
        "Diagnostic logs and crash reports. Safe to delete; apps write new ones.",
        Some(logs), vec![lib("Logs")], false, Some("system"));
    let named = caches + support + containers + developer + logs + own_category;
    part("other-library", "Other ~/Library",
        "Preferences, keychains, fonts, Safari data and everything else apps keep in your Library folder.",
        Some(library.saturating_sub(named)), vec![l.clone()], false, Some("folders"));

    if let Some(hidden) = hidden_home_dirs {
        let total = hidden.iter().map(|(_, s)| s).sum();
        part("hidden-tools", "Developer tools in hidden folders",
            "Folders like ~/.npm, ~/.cache, ~/.cargo, ~/.nvm, ~/.docker in your home folder. Finder hides them, so macOS counts them as System Data.",
            Some(total), hidden.into_iter().map(|(p, _)| p).collect(), false, Some("developer"));
    }

    if let Some(d) = darwin_user_dir() {
        part("user-temp", "Temporary files & system caches",
            "Your per-user folder in /var/folders: temp files and caches of macOS services and apps. macOS cleans it up itself, partly on restart.",
            Some(size(&d)), vec![d], true, None);
    }
    let sys_lib = PathBuf::from("/Library");
    part("system-library", "System-wide app data (/Library)",
        "Data shared by all users: Command Line Tools, printer drivers, app helpers, system-wide caches.",
        Some(size(&sys_lib)), vec![sys_lib], false, None);
    let vm = PathBuf::from("/System/Volumes/VM");
    part("swap", "Swap & sleep image",
        "Memory macOS moved to disk, plus the sleep image. Managed by macOS; shrinks after a restart.",
        volume_used(&vm), vec![vm], true, None);
    let pre = PathBuf::from("/System/Volumes/Preboot");
    let upd = PathBuf::from("/System/Volumes/Update");
    let support_vols = match (volume_used(&pre), volume_used(&upd)) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    };
    part("macos-support", "macOS boot & update files",
        "Boot files, recovery and downloaded macOS updates, on macOS's own volumes. Managed by macOS.",
        support_vols, vec![pre, upd], true, None);

    if let Some(n) = tm_snapshots.filter(|n| *n > 0) {
        parts.push(SystemPart {
            id: "tm-snapshots",
            name: "Local Time Machine snapshots",
            explanation: "Hourly snapshots kept on this disk until Time Machine backs up. macOS deletes them when it needs space.",
            size: None,
            paths: Vec::new(),
            managed_by_macos: true,
            screen: Some("system"),
            note: Some(format!("{n} snapshot(s); macOS doesn't report their size")),
            top: Vec::new(),
        });
    }
    parts.retain(|p| p.size != Some(0));
    parts.sort_by_key(|p| std::cmp::Reverse(p.size.unwrap_or(0)));
    parts
}
