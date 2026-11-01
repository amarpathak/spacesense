//! Data-driven cleanup rules (PLAN.md §4). One TOML file per rule in
//! `core/catalog/`, embedded at build time; a directory can be loaded instead.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/catalog_files.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// Pure cache: deleting costs only re-download / rebuild time.
    Cache,
    /// Build output that the toolchain regenerates.
    Regenerable,
    /// Worth a human look: big re-download, or may hold something wanted.
    Review,
    /// Shown, never cleaned by SpaceSense.
    NeverAuto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fallback {
    /// No automatic action without the official command.
    None,
    /// Delete what's inside the directory, keep the directory.
    DeleteContents,
    /// Delete the directory itself.
    DeleteDir,
    /// Move to the Trash (recoverable).
    Trash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    /// Default locations (`~` expanded). All that exist are counted.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Environment variable that overrides the location.
    pub env: Option<String>,
    /// Appended to the env var's value (e.g. CARGO_HOME + "registry").
    pub env_suffix: Option<String>,
    /// Command that prints the location; used when no default path exists.
    pub path_cmd: Option<Vec<String>>,
    /// Binary the official command needs.
    pub tool: Option<String>,
    /// Extra places to look for `tool` besides $PATH.
    #[serde(default)]
    pub tool_paths: Vec<String>,
    /// App that owns the data; if it is missing, the data is a leftover.
    pub app_bundle: Option<String>,
    /// Process names that must not be running while cleaning.
    #[serde(default)]
    pub quit_apps: Vec<String>,
    pub safety: Safety,
    pub official_cmd: Option<Vec<String>>,
    pub dry_run_cmd: Option<Vec<String>>,
    pub fallback: Fallback,
    /// Action when `app_bundle` is gone (leftover data).
    pub leftover_fallback: Option<Fallback>,
    /// Size excludes other rules' paths nested inside it.
    #[serde(default)]
    pub generic: bool,
    /// Report the biggest children individually.
    #[serde(default)]
    pub itemize: bool,
    /// Sub-paths whose sizes are reported as details.
    #[serde(default)]
    pub details: Vec<String>,
    /// Handled by code instead of paths (e.g. "tm-snapshots").
    pub special: Option<String>,
    pub comes_back: String,
    pub explanation: String,
    pub leftover_explanation: Option<String>,
    #[serde(default)]
    pub docs_url: String,
    pub verified: String,
}

pub fn load_embedded() -> Vec<Rule> {
    EMBEDDED
        .iter()
        .map(|(name, src)| parse(name, src).expect("embedded catalog is valid (checked by tests)"))
        .collect()
}

pub fn load_dir(dir: &Path) -> io::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    for f in files {
        let src = std::fs::read_to_string(&f)?;
        rules.push(
            parse(&f.display().to_string(), &src)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        );
    }
    Ok(rules)
}

pub fn parse(name: &str, src: &str) -> Result<Rule, String> {
    let r: Rule = toml::from_str(src).map_err(|e| format!("{name}: {e}"))?;
    let stem = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .trim_end_matches(".toml");
    if r.id != stem {
        return Err(format!("{name}: id {:?} must match the file name", r.id));
    }
    if r.paths.is_empty() && r.special.is_none() {
        return Err(format!("{name}: needs paths or special"));
    }
    Ok(r)
}

pub fn expand_tilde(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

/// Where a rule's data was found, and how.
#[derive(Clone, Debug, Serialize)]
pub struct Resolved {
    pub paths: Vec<PathBuf>,
    /// "env", "default" or "command".
    pub source: &'static str,
}

impl Rule {
    pub fn resolve(&self, home: &Path, tools: &Tools) -> Resolved {
        if let Some(var) = &self.env {
            if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                let mut p = PathBuf::from(v);
                if let Some(s) = &self.env_suffix {
                    p.push(s);
                }
                return Resolved {
                    paths: vec![p],
                    source: "env",
                };
            }
        }
        let defaults: Vec<PathBuf> = self
            .paths
            .iter()
            .map(|p| expand_tilde(p, home))
            .filter(|p| p.symlink_metadata().is_ok())
            .collect();
        if !defaults.is_empty() {
            return Resolved {
                paths: defaults,
                source: "default",
            };
        }
        if let Some(cmd) = &self.path_cmd {
            if let Some(bin) = tools.find(&cmd[0], &[]) {
                if let Some(out) = run_capture(&bin, &cmd[1..], Duration::from_secs(5)) {
                    let p = PathBuf::from(out.trim());
                    if p.is_absolute() && p.exists() {
                        return Resolved {
                            paths: vec![p],
                            source: "command",
                        };
                    }
                }
            }
        }
        Resolved {
            paths: Vec::new(),
            source: "default",
        }
    }

    pub fn tool_path(&self, tools: &Tools) -> Option<PathBuf> {
        let t = self.tool.as_ref()?;
        tools.find(t, &self.tool_paths)
    }

    /// The owning app is known and missing: what's left is leftover data.
    pub fn is_leftover(&self) -> bool {
        self.app_bundle
            .as_ref()
            .is_some_and(|a| !Path::new(a).exists())
    }
}

/// Finds command-line tools. A GUI app gets a minimal $PATH, so common
/// install locations are searched too.
pub struct Tools {
    dirs: Vec<PathBuf>,
}

impl Tools {
    pub fn new(home: &Path) -> Tools {
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            dirs.push(d.into());
        }
        for d in [".cargo/bin", ".bun/bin", ".local/bin", "go/bin"] {
            dirs.push(home.join(d));
        }
        Tools { dirs }
    }

    pub fn find(&self, name: &str, extra: &[String]) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt;
        let exe = |p: &Path| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        };
        self.dirs
            .iter()
            .map(|d| d.join(name))
            .chain(extra.iter().map(PathBuf::from))
            .find(|p| exe(p))
    }
}

/// Run a command, return stdout if it exits 0 within `timeout`.
pub fn run_capture(bin: &Path, args: &[String], timeout: Duration) -> Option<String> {
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                if !st.success() {
                    return None;
                }
                let mut s = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut s).ok()?;
                return Some(s);
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Names of running processes (for `quit_apps`).
pub fn running_process_names() -> Vec<String> {
    let out = Command::new("/bin/ps").args(["-Axo", "comm="]).output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().rsplit('/').next().unwrap_or("").to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses_and_is_consistent() {
        let rules = load_embedded();
        assert!(rules.len() >= 30);
        let mut ids = std::collections::HashSet::new();
        for r in &rules {
            assert!(ids.insert(r.id.clone()), "duplicate id {}", r.id);
            if r.official_cmd.is_some() {
                assert!(r.tool.is_some(), "{}: official_cmd needs tool", r.id);
            }
            if r.safety == Safety::NeverAuto {
                assert!(
                    r.official_cmd.is_none() && r.fallback == Fallback::None,
                    "{}",
                    r.id
                );
            }
            for p in &r.paths {
                assert!(p.starts_with("~/"), "{}: paths must be under ~ ({p})", r.id);
            }
            assert!(
                !r.explanation.is_empty() && !r.comes_back.is_empty(),
                "{}",
                r.id
            );
        }
    }
}
