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
}

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
        let has_action = r.rule.official_cmd.is_some() || r.rule.fallback != Fallback::None || r.leftover;
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

    if explicit {
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
    if r.rule.special.is_some() {
        return Err("not supported yet (needs an administrator prompt)".into());
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
                Fallback::None => unreachable!(),
            });
        }
    }
    if r.safety == Safety::Review && !r.leftover {
        warnings.push(r.rule.comes_back.clone());
    }
    Ok(Action {
        target: r.id.clone(),
        source: r.id.clone(),
        title: r.name.clone(),
        safety: r.safety,
        bytes: r.size,
        steps,
        warnings,
        blockers: r.running_apps.iter().map(|a| format!("quit {a} first")).collect(),
        quit_apps: r.rule.quit_apps.clone(),
        suggested: false,
    })
}
