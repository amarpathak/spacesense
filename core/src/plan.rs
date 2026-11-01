//! Cleanup planning (PLAN.md §5). Planning never changes anything; `clean.rs`
//! executes a plan, and only for targets a person picked and confirmed.
//!
//! Three ways to build a plan:
//! - explicit targets (what a person selected): rule ids, project paths,
//!   single build-folder paths, large-file paths;
//! - `candidates`: everything that *could* be cleaned, for the app to list
//!   with nothing selected;
//! - neither: the suggested selection shown by `clean --dry-run` (caches,
//!   regenerable data, projects idle 30+ days). It is a suggestion only.

use crate::catalog::{Fallback, Safety};
use crate::report::{Report, RuleReport};
use crate::safety;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Projects untouched for longer than this are suggested.
pub const STALE_DAYS: i64 = 30;
/// Projects active more recently than this are never cleaned unless forced.
pub const ACTIVE_DAYS: i64 = 14;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Step {
    /// Run the tool's own cleanup command. `measure`: folders it frees space in.
    Run {
        argv: Vec<String>,
        dry_run_argv: Option<Vec<String>>,
        measure: Vec<PathBuf>,
    },
    DeleteContents { path: PathBuf },
    DeleteDir { path: PathBuf },
    Trash { path: PathBuf },
    /// Delete files (not folders) last modified more than `days` ago.
    DeleteOld { path: PathBuf, days: u32 },
}

/// Prefix for a folder picked in the Folders screen ("folder:/Users/me/...").
pub const FOLDER: &str = "folder:";

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    /// What to pass back as a target to select exactly this action.
    pub target: String,
    /// Rule id, "project" or "large-file".
    pub source: String,
    pub title: String,
    pub safety: Safety,
    /// Estimated bytes freed (re-measured when cleaning).
    pub bytes: u64,
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Must be resolved before running (app still running…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    /// Processes re-checked right before running.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quit_apps: Vec<String>,
    /// Shown in "suggested" (dry-run default) selections.
    pub suggested: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub target: String,
    pub bytes: u64,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub skipped: Vec<Skipped>,
    pub total_bytes: u64,
}

#[derive(Debug, Default, Clone)]
pub struct Selection {
    /// Explicit targets (see module docs). Empty = suggested selection.
    pub targets: Vec<String>,
    /// Allow projects active within ACTIVE_DAYS.
    pub include_active: bool,
    /// List every cleanable item instead (targets are ignored).
    pub candidates: bool,
}

pub fn build(report: &Report, sel: &Selection) -> Plan {
    let now = report.generated_at;
    let protected = protected_paths(report);
    let mut plan = Plan::default();
    let explicit = !sel.targets.is_empty() && !sel.candidates;
    let mut matched = vec![false; sel.targets.len()];
    let named_pos = |t: &dyn Fn(&str) -> bool| if explicit { sel.targets.iter().position(|s| t(s)) } else { None };

    for r in &report.rules {
        let named = named_pos(&|t| t == r.id);
        if let Some(i) = named {
            matched[i] = true;
        }
        let suggested = matches!(r.safety, Safety::Cache | Safety::Regenerable) && !r.rule.generic;
        let wanted = named.is_some()
            || (sel.candidates && r.safety != Safety::NeverAuto && !r.rule.generic)
            || (!explicit && !sel.candidates && suggested);
        if !wanted || (!r.found && named.is_none()) {
            continue;
        }
        if r.safety == Safety::NeverAuto {
            if named.is_some() {
                plan.skipped.push(skip(&r.id, r.size, "never cleaned (shown for information only)"));
            }
            continue;
        }
        if r.size == 0 && r.rule.special.is_none() {
            if named.is_some() {
                plan.skipped.push(skip(&r.id, 0, "nothing found"));
            }
            continue;
        }
        let has_action = r.rule.official_cmd.is_some()
            || r.rule.fallback != Fallback::None
            || r.leftover
            || (r.rule.special.as_deref() == Some("tm-snapshots") && tm_snapshot_count() > 0);
        if named.is_none() && !has_action {
            continue; // nothing SpaceSense can do; shown in `report`
        }
        match rule_action(r, &report.home, &protected) {
            Ok(mut a) => {
                a.suggested = suggested;
                plan.actions.push(a)
            }
            Err(reason) => {
                if named.is_some() || !sel.candidates {
                    plan.skipped.push(skip(&r.id, r.size, &reason))
                }
            }
        }
    }

    for p in &report.projects {
        // A project, its group (main repo of worktrees), or one of its build folders.
        let named_project = named_pos(&|t| Path::new(t) == p.path || Path::new(t) == p.group);
        let named_artifacts: Vec<usize> = if explicit {
            p.artifacts
                .iter()
                .enumerate()
                .filter(|(_, a)| sel.targets.iter().any(|t| Path::new(t) == a.path))
                .map(|(i, _)| i)
                .collect()
        } else {
            Vec::new()
        };
        if let Some(i) = named_project {
            matched[i] = true;
        }
        for &ai in &named_artifacts {
            if let Some(i) = sel.targets.iter().position(|t| Path::new(t) == p.artifacts[ai].path) {
                matched[i] = true;
            }
        }
        if explicit && named_project.is_none() && named_artifacts.is_empty() {
            continue;
        }
        let days = (now - p.last_active) / 86_400;
        let label = p.path.display().to_string();
        if days < ACTIVE_DAYS && !sel.include_active {
            if !sel.candidates {
                plan.skipped.push(skip(
                    &label,
                    p.artifact_bytes,
                    &format!("active {days} days ago (protected for {ACTIVE_DAYS} days)"),
                ));
            }
            continue;
        }
        let suggested = days >= STALE_DAYS;
        if !explicit && !sel.candidates && !suggested {
            plan.skipped.push(skip(&label, p.artifact_bytes, &format!("active {days} days ago (suggested after {STALE_DAYS})")));
            continue;
        }
        let mut warnings = Vec::new();
        if let Some(g) = &p.git {
            if g.dirty == Some(true) {
                warnings.push("repo has uncommitted changes (these folders are git-ignored, so they aren't part of them)".into());
            }
            if let Some(main) = &g.worktree_of {
                warnings.push(format!("git worktree of {}", main.display()));
            }
        }
        let mut steps = Vec::new();
        let mut bytes = 0;
        let mut regen: Vec<&str> = Vec::new();
        for (ai, a) in p.artifacts.iter().enumerate() {
            let picked = named_artifacts.contains(&ai);
            if !named_artifacts.is_empty() && named_project.is_none() && !picked {
                continue; // only the build folders that were picked
            }
            // Review-level build folders (dist/, venvs) only when picked by name.
            if a.safety == Safety::Review && !picked {
                continue;
            }
            if a.git_ignored == Some(false) {
                if !sel.candidates {
                    plan.skipped.push(skip(
                        &a.path.display().to_string(),
                        a.size,
                        "not ignored by git: may contain tracked or uncommitted files",
                    ));
                }
                continue;
            }
            match safety::validate(&a.path, &report.home, &protected) {
                Ok(real) => {
                    steps.push(if a.safety == Safety::Review { Step::Trash { path: real } } else { Step::DeleteDir { path: real } });
                    bytes += a.size;
                    if !regen.contains(&a.regenerate) {
                        regen.push(a.regenerate);
                    }
                }
                Err(e) => warnings.push(format!("{}: {e}", a.name)),
            }
        }
        if steps.is_empty() {
            continue;
        }
        warnings.push(format!("rebuilt by: {}", regen.join(", ")));
        let target = if named_project.is_none() && named_artifacts.len() == 1 {
            p.artifacts[named_artifacts[0]].path.display().to_string()
        } else {
            label.clone()
        };
        plan.actions.push(Action {
            target,
            source: "project".into(),
            title: format!("{label} (last active {days} days ago)"),
            safety: Safety::Regenerable,
            bytes,
            steps,
            warnings,
            blockers: Vec::new(),
            quit_apps: Vec::new(),
            suggested,
        });
    }

    // Large files: only ever to the Trash, only when picked (or listed as candidates).
    for f in &report.large_files {
        let named = named_pos(&|t| Path::new(t) == f.path);
        if let Some(i) = named {
            matched[i] = true;
        }
        if named.is_none() && !sel.candidates {
            continue;
        }
        match safety::validate(&f.path, &report.home, &protected) {
            Ok(real) => plan.actions.push(Action {
                target: f.path.display().to_string(),
                source: "large-file".into(),
                title: f.path.display().to_string(),
                safety: Safety::Review,
                bytes: f.size,
                steps: vec![Step::Trash { path: real }],
                warnings: vec!["your file: moved to the Trash, so you can put it back".into()],
                blockers: Vec::new(),
                quit_apps: Vec::new(),
                suggested: false,
            }),
            Err(e) => plan.skipped.push(skip(&f.path.display().to_string(), f.size, &e.to_string())),
        }
    }

    // Folders picked in the folder browser.
    if explicit {
        for (i, t) in sel.targets.iter().enumerate() {
            let Some(p) = t.strip_prefix(FOLDER) else { continue };
            matched[i] = true;
            match folder_action(report, Path::new(p), &protected) {
                Ok(a) => plan.actions.push(a),
                Err(why) => plan.skipped.push(skip(p, report.size_of(Path::new(p)).map(|n| n.size).unwrap_or(0), &why)),
            }
        }
        for (t, m) in sel.targets.iter().zip(matched) {
            if !m {
                plan.skipped.push(skip(t, 0, "no rule, project or file matches this target"));
            }
        }
    }
    plan.total_bytes = plan.actions.iter().map(|a| a.bytes).sum();
    plan
}

/// Never-auto locations: refused by path validation everywhere.
pub fn protected_paths(report: &Report) -> Vec<PathBuf> {
    report
        .rules
        .iter()
        .filter(|r| r.safety == Safety::NeverAuto)
        .flat_map(|r| r.paths.iter().map(|p| p.path.clone()))
        .collect()
}

fn skip(target: &str, bytes: u64, reason: &str) -> Skipped {
    Skipped { target: target.into(), bytes, reason: reason.into() }
}

fn rule_action(r: &RuleReport, home: &Path, protected: &[PathBuf]) -> Result<Action, String> {
    if r.safety == Safety::NeverAuto {
        return Err("never cleaned (shown for information only)".into());
    }
    if r.rule.special.as_deref() == Some("tm-snapshots") {
        let n = tm_snapshot_count();
        if n == 0 {
            return Err("no local Time Machine snapshots".into());
        }
        return Ok(Action {
            target: r.id.clone(),
            source: r.id.clone(),
            title: format!("{} ({n})", r.name),
            safety: r.safety,
            bytes: 0,
            steps: vec![Step::Run {
                // macOS shows its own administrator password prompt.
                argv: vec![
                    "/usr/bin/osascript".into(),
                    "-e".into(),
                    "do shell script \"/usr/bin/tmutil deletelocalsnapshots /\" with administrator privileges".into(),
                ],
                dry_run_argv: None,
                measure: vec![],
            }],
            warnings: vec![
                "macOS asks for your administrator password.".into(),
                "Their size can't be measured beforehand; check free space after.".into(),
                r.rule.comes_back.clone(),
            ],
            blockers: vec![],
            quit_apps: vec![],
            suggested: false,
        });
    }
    if r.rule.special.is_some() {
        return Err("not supported yet".into());
    }
    if r.rule.generic {
        return Err("pick individual apps' caches (generic location)".into());
    }
    let mut warnings = Vec::new();
    let use_official = r.rule.official_cmd.is_some() && r.tool.is_some() && !r.leftover;
    let fallback = if r.leftover { r.rule.leftover_fallback.unwrap_or(r.rule.fallback) } else { r.rule.fallback };
    let mut steps = Vec::new();
    if use_official {
        let mut argv = r.rule.official_cmd.clone().unwrap();
        argv[0] = r.tool.as_ref().unwrap().display().to_string();
        let dry = r.rule.dry_run_cmd.clone().map(|mut d| {
            d[0] = argv[0].clone();
            d
        });
        steps.push(Step::Run { argv, dry_run_argv: dry, measure: r.paths.iter().map(|p| p.path.clone()).collect() });
    } else {
        if fallback == Fallback::None {
            let why = if r.leftover {
                "owning app is uninstalled and no fallback is defined".to_string()
            } else if let (Some(_), Some(tool)) = (&r.rule.official_cmd, &r.rule.tool) {
                format!("`{tool}` is not installed, and deleting by hand isn't safe for this data")
            } else {
                "no automatic cleanup for this item; see its explanation".to_string()
            };
            return Err(why);
        }
        if r.rule.official_cmd.is_some() && !r.leftover {
            warnings.push(format!("`{}` not found; deleting the cache directly", r.rule.tool.as_deref().unwrap_or("?")));
        }
        if r.leftover {
            warnings.push(r.rule.leftover_explanation.clone().unwrap_or_else(|| "owning app is not installed".into()));
        }
        for p in &r.paths {
            let real = safety::validate(&p.path, home, protected).map_err(|e| format!("{}: {e}", p.path.display()))?;
            steps.push(match fallback {
                Fallback::DeleteContents => Step::DeleteContents { path: real },
                Fallback::DeleteDir => Step::DeleteDir { path: real },
                Fallback::Trash => Step::Trash { path: real },
                Fallback::DeleteOld => Step::DeleteOld { path: real, days: r.rule.older_than_days.unwrap_or(30) },
                Fallback::None => unreachable!(),
            });
        }
    }
    let mut bytes = r.size;
    if fallback == Fallback::DeleteOld && !use_official {
        let days = r.rule.older_than_days.unwrap_or(30);
        bytes = r.paths.iter().map(|p| old_files_bytes(&p.path, days)).sum();
        warnings.push(format!("only files older than {days} days"));
    }
    if r.safety == Safety::Review && !r.leftover {
        warnings.push(r.rule.comes_back.clone());
    }
    Ok(Action {
        target: r.id.clone(),
        source: r.id.clone(),
        title: r.name.clone(),
        safety: r.safety,
        bytes,
        steps,
        warnings,
        blockers: r.running_apps.iter().map(|a| format!("quit {a} first")).collect(),
        quit_apps: r.rule.quit_apps.clone(),
        suggested: false,
    })
}

/// Bytes of files under `dir` last modified more than `days` ago.
pub fn old_files_bytes(dir: &Path, days: u32) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(days as u64 * 86_400);
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.path().symlink_metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else if m.modified().map(|t| t < cutoff).unwrap_or(false) {
                total += m.blocks() * 512;
            }
        }
    }
    total
}

fn tm_snapshot_count() -> usize {
    std::process::Command::new("/usr/bin/tmutil")
        .args(["listlocalsnapshots", "/"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("com.apple.TimeMachine")).count())
        .unwrap_or(0)
}

/// Process names of the app that owns `~/Library/<kind>/<name>`, for the
/// "quit it first" check. Folder names usually match; a few vendors don't.
fn owner_processes(name: &str) -> Vec<String> {
    let mapped = match name {
        "Google" | "com.google.Chrome" => "Google Chrome",
        "BraveSoftware" | "com.brave.Browser" => "Brave Browser",
        "Microsoft Edge" | "com.microsoft.edgemac" => "Microsoft Edge",
        "Code" | "com.microsoft.VSCode" => "Code",
        "Firefox" | "Mozilla" | "org.mozilla.firefox" => "firefox",
        "com.tinyspeck.slackmacgap" => "Slack",
        other if !other.contains('.') => other,
        _ => return Vec::new(),
    };
    vec![mapped.to_string()]
}

/// A folder picked in the Folders screen. The verdict decides the action:
/// cache / rebuildable → delete; review → Trash; don't-delete or unknown → refused.
fn folder_action(report: &Report, path: &Path, protected: &[PathBuf]) -> Result<Action, String> {
    let rel = path.strip_prefix(&report.home).map_err(|_| "outside your home folder".to_string())?;
    let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
    const TOP: &[&str] = &["Library", "Desktop", "Documents", "Downloads", "Pictures", "Movies", "Music", "Applications", "Public"];
    if parts.is_empty() || (parts.len() == 1 && TOP.contains(&parts[0])) || (parts.len() == 2 && parts[0] == "Library") {
        return Err("this folder holds many apps' or your own data: pick a folder inside it".into());
    }
    let node = report.size_of(path).ok_or("not in the last scan; rescan first")?;
    if node.error.as_deref() == Some(crate::SKIPPED) {
        return Err("cloud storage isn't cleaned by SpaceSense".into());
    }
    let info = report.explain(path);
    let step_for = |real: PathBuf| match info.safety {
        Some(Safety::Cache) | Some(Safety::Regenerable) => Ok(Step::DeleteDir { path: real }),
        Some(Safety::Review) => Ok(Step::Trash { path: real }),
        Some(Safety::NeverAuto) => Err(format!("{}: don't delete", info.label)),
        None => Err("SpaceSense doesn't know what this folder is; use Finder if you're sure".to_string()),
    };
    let real = safety::validate(path, &report.home, protected).map_err(|e| e.to_string())?;
    let step = step_for(real)?;
    let mut warnings = Vec::new();
    if matches!(step, Step::Trash { .. }) {
        warnings.push("moved to the Trash: you can put it back; the space returns when you empty the Trash".into());
    } else {
        warnings.push("deleted permanently; the app rebuilds it when needed".into());
    }
    let inside_projects = report.projects.iter().filter(|p| p.path.starts_with(path)).count();
    if inside_projects > 0 {
        warnings.push(format!("contains {inside_projects} project(s), including their source code"));
    }
    if !info.explanation.is_empty() {
        warnings.push(info.explanation.clone());
    }
    let quit_apps = match parts.as_slice() {
        ["Library", "Application Support" | "Caches" | "Containers" | "Logs" | "Saved Application State", app, ..] => owner_processes(app),
        _ => Vec::new(),
    };
    let running = crate::catalog::running_process_names();
    let blockers = quit_apps.iter().filter(|a| running.iter().any(|p| p == *a)).map(|a| format!("quit {a} first")).collect();
    Ok(Action {
        target: format!("{FOLDER}{}", path.display()),
        source: "folder".into(),
        title: path.display().to_string(),
        safety: info.safety.unwrap_or(Safety::Review),
        bytes: node.size,
        steps: vec![step],
        warnings,
        blockers,
        quit_apps,
        suggested: false,
    })
}
