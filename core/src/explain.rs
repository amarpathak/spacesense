//! "What is this folder, and is it safe to delete?" for any folder in the scan
//! tree. Used by the folder browser. Catalog rules and project artifacts win;
//! otherwise macOS conventions for ~/Library and the user folders decide.

use crate::catalog::Safety;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Explanation {
    /// Short label: "App cache", "App data", "Your files", "System data"…
    pub label: String,
    /// None = unknown: no verdict.
    pub safety: Option<Safety>,
    pub explanation: String,
    /// Catalog rule id when one matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    /// Bundle id or app name hint that the app can resolve to an installed app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_hint: Option<String>,
}

fn ex(label: &str, safety: Option<Safety>, explanation: &str, owner: Option<&str>) -> Explanation {
    Explanation {
        label: label.into(),
        safety,
        explanation: explanation.into(),
        rule_id: None,
        owner_hint: owner.map(String::from),
    }
}

/// Folders under ~/Library that hold personal or system data: never delete.
const LIBRARY_NEVER: &[(&str, &str)] = &[
    ("Mail", "Your email and attachments (Mail app)."),
    ("Messages", "Your iMessage/SMS history and attachments."),
    ("Photos", "Photos library data."),
    ("Mobile Documents", "iCloud Drive. Deleting here deletes from iCloud on all your devices."),
    ("CloudStorage", "Dropbox, Google Drive, OneDrive and other cloud folders. Deleting here may delete in the cloud."),
    ("Keychains", "Your saved passwords and certificates."),
    ("Preferences", "Settings of every app. Small; deleting resets apps."),
    ("Accounts", "Internet accounts configured on this Mac."),
    ("Cookies", "Website logins and cookies."),
    ("Safari", "Safari bookmarks, history and reading list."),
    ("Calendars", "Your calendars."),
    ("Metadata", "Spotlight and system metadata. macOS manages it."),
    ("Biome", "macOS usage data. macOS manages it."),
    ("Keyboard", "Keyboard dictionaries."),
    ("Sharing", "System sharing data."),
    ("Suggestions", "Siri suggestions data. macOS manages it."),
    ("Daemon Containers", "System services' data. macOS manages it."),
];

pub fn explain(path: &Path, home: &Path) -> Explanation {
    let Ok(rel) = path.strip_prefix(home) else {
        return ex("Outside your home folder", None, "", None);
    };
    let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
    let last = parts.last().copied().unwrap_or("");
    match parts.as_slice() {
        [] => ex("Your home folder", None, "Everything that belongs to your user account.", None),
        ["Library"] => ex(
            "App & system data",
            Some(Safety::Review),
            "Where apps and macOS keep caches, settings and data. Never delete Library itself; look inside for the big folders.",
            None,
        ),
        ["Library", "Caches"] => ex(
            "App caches",
            Some(Safety::Cache),
            "Caches of all apps. Each app's cache is safe to delete while that app is quit; apps rebuild them.",
            None,
        ),
        ["Library", "Caches", app, ..] => ex(
            "App cache",
            Some(Safety::Cache),
            "Temporary data the app can download or rebuild. Safe to delete while the app is quit; it may be slower at first.",
            Some(app),
        ),
        ["Library", "Logs", ..] => ex(
            "Logs",
            Some(Safety::Cache),
            "Diagnostic logs. Safe to delete; apps write new ones.",
            parts.get(2).copied(),
        ),
        ["Library", "Saved Application State", ..] => ex(
            "Window state",
            Some(Safety::Cache),
            "Which windows apps had open. Safe to delete; apps just open fresh.",
            None,
        ),
        ["Library", "Application Support"] => ex(
            "App data",
            Some(Safety::Review),
            "Data of all apps: settings, databases, downloaded content. Look per app.",
            None,
        ),
        ["Library", "Application Support", app, ..] => ex(
            "App data",
            Some(Safety::Review),
            "The app's own data: settings, databases, downloads, sometimes your documents. Deleting may reset the app or lose data. If the app is uninstalled, this is leftover.",
            Some(app),
        ),
        ["Library", "Containers"] => ex(
            "Sandboxed app data",
            Some(Safety::Review),
            "Each sandboxed app (App Store apps, Docker, …) keeps all its data in its own container. Look per app.",
            None,
        ),
        ["Library", "Containers", app, ..] => ex(
            "Sandboxed app data",
            Some(Safety::Review),
            "Everything this app stores: settings, caches and documents saved inside the app. Deleting resets the app and can lose data. If the app is uninstalled, it's leftover.",
            Some(app),
        ),
        ["Library", "Developer", ..] => ex(
            "Developer tools data",
            Some(Safety::Review),
            "Xcode, simulators and SDK data. See the Developer screen for what's safe.",
            None,
        ),
        ["Library", "Group Containers"] => ex(
            "Shared app data",
            Some(Safety::Review),
            "Data shared between apps of one developer (Office, WhatsApp, Notes…). Can include documents and chat history. Look per app.",
            None,
        ),
        ["Library", "Group Containers", group, ..] => ex(
            "Shared app data",
            Some(Safety::Review),
            "Data shared between apps of one developer. Can include documents and chat history (WhatsApp, Office, Notes).",
            Some(group),
        ),
        ["Library", name] if LIBRARY_NEVER.iter().any(|(n, _)| n == name) => {
            let why = LIBRARY_NEVER.iter().find(|(n, _)| n == name).unwrap().1;
            ex("Personal or system data", Some(Safety::NeverAuto), &format!("{why} Don't delete this folder."), None)
        }
        ["Library", name, ..] if LIBRARY_NEVER.iter().any(|(n, _)| n == name) => ex(
            "Personal or system data",
            Some(Safety::NeverAuto),
            "Part of personal or system data managed by macOS. Don't delete.",
            None,
        ),
        ["Library", ..] => ex(
            "App & system data",
            Some(Safety::Review),
            "Data apps or macOS keep here. Check what it belongs to before deleting.",
            None,
        ),
        [top, ..] if ["Desktop", "Documents", "Downloads", "Pictures", "Movies", "Music"].contains(top) => {
            if parts.len() == 1 {
                ex(
                    "Your files",
                    Some(Safety::Review),
                    "Your own files. Only you can tell what's still needed; SpaceSense only ever moves these to the Trash.",
                    None,
                )
            } else if last.ends_with(".photoslibrary") {
                ex("Photos library", Some(Safety::NeverAuto), "Your Photos library. Manage it in the Photos app, never by deleting files.", None)
            } else {
                ex("Your files", Some(Safety::Review), "Your own files. Review before deleting; prefer moving to the Trash.", None)
            }
        }
        [top, ..] if [".ssh", ".gnupg", ".aws", ".azure", ".kube", ".password-store", ".docker", ".gcloud"].contains(top) => ex(
            "Keys and credentials",
            Some(Safety::NeverAuto),
            "SSH keys, signing keys or cloud credentials. Losing them can lock you out of servers and accounts. Don't delete.",
            None,
        ),
        [top, ..] if top.starts_with('.') => ex(
            "Tool data",
            Some(Safety::Review),
            "Hidden folder used by a command-line tool or app (settings, caches, installed versions). Check which tool before deleting.",
            Some(top.trim_start_matches('.')),
        ),
        _ => ex("Folder", None, "", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_conventions() {
        let h = Path::new("/Users/me");
        let s = |p: &str| explain(&h.join(p), h).safety;
        assert_eq!(s("Library/Caches/com.spotify.client"), Some(Safety::Cache));
        assert_eq!(s("Library/Application Support/Slack"), Some(Safety::Review));
        assert_eq!(s("Library/Containers/com.docker.docker"), Some(Safety::Review));
        assert_eq!(s("Library/Mail"), Some(Safety::NeverAuto));
        assert_eq!(s("Library/Mobile Documents/com~apple~CloudDocs"), Some(Safety::NeverAuto));
        assert_eq!(s("Pictures/Photos Library.photoslibrary"), Some(Safety::NeverAuto));
        assert_eq!(s("Downloads/x.zip"), Some(Safety::Review));
        assert_eq!(
            explain(&h.join("Library/Containers/com.docker.docker"), h).owner_hint.as_deref(),
            Some("com.docker.docker")
        );
    }
}
