//! Cleanup planning (PLAN.md §5). M1 only *plans*: nothing here deletes
//! anything. `clean --dry-run` prints the plan; M3 will execute it.

use crate::catalog::{Fallback, Safety};
use crate::report::{Report, RuleReport};
use crate::safety;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Projects untouched for longer than this are preselected.
pub const STALE_DAYS: i64 = 30;
/// Projects active more recently than this are never cleaned unless forced.
pub const ACTIVE_DAYS: i64 = 14;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Step {
    /// Run the tool's own cleanup command.
    Run {
        argv: Vec<String>,
        dry_run_argv: Option<Vec<String>>,
    },
    DeleteContents {
        path: PathBuf,
    },
    DeleteDir {
        path: PathBuf,
    },
    Trash {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    /// Rule id, or "project".
    pub source: String,
    pub title: String,
    pub safety: Safety,
    /// Estimated bytes freed (re-measured after cleaning in M3).
    pub bytes: u64,
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Must be resolved before running (app still running...).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
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

#[derive(Debug, Default)]
pub struct Selection {
    /// Explicit targets: rule ids or project paths. Empty = the default
    /// selection (caches, regenerable rules, stale projects).
    pub targets: Vec<String>,
    /// Allow projects active within ACTIVE_DAYS.
    pub include_active: bool,
}

pub fn build(report: &Report, sel: &Selection) -> Plan {
    let now = report.generated_at;
    let protected: Vec<PathBuf> = report
        .rules
        .iter()
        .filter(|r| r.safety == Safety::NeverAuto)
        .flat_map(|r| r.rules_paths())
        .collect();
    let mut plan = Plan::default();
    let explicit = !sel.targets.is_empty();
    let mut matched = vec![false; sel.targets.len()];

    for r in &report.rules {
        let named = sel.targets.iter().position(|t| t == &r.id);
        if let Some(i) = named {
            matched[i] = true;
        }
        let wanted = match named {
            Some(_) => true,
            None => {
                !explicit
                    && matches!(r.safety, Safety::Cache | Safety::Regenerable)
                    && !r.rule.generic
            }
        };
        if !wanted || (!r.found && named.is_none()) {
            continue;
        }
        if r.safety == Safety::NeverAuto {
            if named.is_some() {
                plan.skipped.push(skip(
                    &r.id,
                    r.size,
                    "never cleaned automatically (shown for information)",
                ));
            }
            continue;
        }
        if r.size == 0 && r.rule.special.is_none() {
            if named.is_some() {
                plan.skipped.push(skip(&r.id, 0, "nothing found"));
            }
            continue;
        }
        let has_action = r.rule.official_cmd.is_some() || r.rule.fallback != Fallback::None;
        if named.is_none() && !has_action {
            continue; // nothing SpaceSense can do automatically; shown in `report`
        }
        match rule_action(r, &report.home, &protected) {
            Ok(a) => plan.actions.push(a),
            Err(reason) => plan.skipped.push(skip(&r.id, r.size, &reason)),
        }
    }

    for p in &report.projects {
        let named = sel
            .targets
            .iter()
            .position(|t| Path::new(t) == p.path || Path::new(t) == p.group);
        if let Some(i) = named {
            matched[i] = true;
        }
        if explicit && named.is_none() {
            continue;
        }
        let days = (now - p.last_active) / 86_400;
        let label = p.path.display().to_string();
        if days < ACTIVE_DAYS && !sel.include_active {
            plan.skipped.push(skip(
                &label,
                p.artifact_bytes,
                &format!("active {days} days ago (protected for {ACTIVE_DAYS} days)"),
            ));
            continue;
        }
        if !explicit && days < STALE_DAYS {
            plan.skipped.push(skip(
                &label,
                p.artifact_bytes,
                &format!("active {days} days ago (preselected after {STALE_DAYS})"),
            ));
            continue;
        }
        let mut warnings = Vec::new();
        if let Some(g) = &p.git {
            if g.dirty == Some(true) {
                warnings.push("repo has uncommitted changes (the folders below are git-ignored, so they aren't part of them)".into());
            }
            if let Some(main) = &g.worktree_of {
                warnings.push(format!("git worktree of {}", main.display()));
            }
        }
        let mut steps = Vec::new();
        let mut bytes = 0;
        let mut regen: Vec<&str> = Vec::new();
        for a in &p.artifacts {
            // Review-level artifacts (dist/, venvs) only when the project is named.
            if a.safety == Safety::Review && !explicit {
                continue;
            }
            if a.git_ignored == Some(false) {
                plan.skipped.push(skip(
                    &a.path.display().to_string(),
                    a.size,
                    "not ignored by git: may contain tracked or uncommitted files",
                ));
                continue;
            }
            match safety::validate(&a.path, &report.home, &protected) {
                Ok(real) => {
                    steps.push(if a.safety == Safety::Review {
                        Step::Trash { path: real }
                    } else {
                        Step::DeleteDir { path: real }
                    });
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
        warnings.push(format!("regenerate with: {}", regen.join(", ")));
        plan.actions.push(Action {
            source: "project".into(),
            title: format!("{} (last active {days} days ago)", label),
            safety: Safety::Regenerable,
            bytes,
            steps,
            warnings,
            blockers: Vec::new(),
        });
    }

    for (t, m) in sel.targets.iter().zip(matched) {
        if !m {
            plan.skipped
                .push(skip(t, 0, "no rule or project matches this target"));
        }
    }
    plan.total_bytes = plan.actions.iter().map(|a| a.bytes).sum();
    plan
}

fn skip(target: &str, bytes: u64, reason: &str) -> Skipped {
    Skipped {
        target: target.into(),
        bytes,
        reason: reason.into(),
    }
}

impl RuleReport {
    fn rules_paths(&self) -> Vec<PathBuf> {
        self.paths.iter().map(|p| p.path.clone()).collect()
    }
}

fn rule_action(r: &RuleReport, home: &Path, protected: &[PathBuf]) -> Result<Action, String> {
    if r.safety == Safety::NeverAuto {
        return Err("never cleaned automatically (shown for information)".into());
    }
    if r.rule.special.is_some() {
        return Err("not supported by the CLI yet (needs an admin prompt, M3)".into());
    }
    if r.rule.generic {
        return Err("pick individual apps' caches (generic location)".into());
    }
    let mut warnings = Vec::new();
    let use_official = r.rule.official_cmd.is_some() && r.tool.is_some() && !r.leftover;
    let fallback = if r.leftover {
        r.rule.leftover_fallback.unwrap_or(r.rule.fallback)
    } else {
        r.rule.fallback
    };
    let mut steps = Vec::new();
    if use_official {
        let mut argv = r.rule.official_cmd.clone().unwrap();
        argv[0] = r.tool.as_ref().unwrap().display().to_string();
        let dry = r.rule.dry_run_cmd.clone().map(|mut d| {
            d[0] = argv[0].clone();
            d
        });
        steps.push(Step::Run {
            argv,
            dry_run_argv: dry,
        });
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
            warnings.push(format!(
                "`{}` not found; falling back to deleting the cache",
                r.rule.tool.as_deref().unwrap_or("?")
            ));
        }
        if r.leftover {
            warnings.push(
                r.rule
                    .leftover_explanation
                    .clone()
                    .unwrap_or_else(|| "owning app is not installed".into()),
            );
        }
        for p in &r.paths {
            let real = safety::validate(&p.path, home, protected)
                .map_err(|e| format!("{}: {e}", p.path.display()))?;
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
    let blockers = r
        .running_apps
        .iter()
        .map(|a| format!("quit {a} first"))
        .collect();
    Ok(Action {
        source: r.id.clone(),
        title: r.name.clone(),
        safety: r.safety,
        bytes: r.size,
        steps,
        warnings,
        blockers,
    })
}
