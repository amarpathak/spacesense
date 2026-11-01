//! Path validation for cleanup targets (PLAN.md §5.6). Every path an action
//! would touch goes through `validate` first.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    NotFound,
    /// Resolves (after symlinks) outside the home directory.
    OutsideHome(PathBuf),
    /// `/`, the home directory itself, or a path that is too short.
    TooBroad(PathBuf),
    /// Inside a never-auto location (iOS backups, Archives, ...).
    Protected(PathBuf),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotFound => write!(f, "path does not exist"),
            Refusal::OutsideHome(p) => {
                write!(f, "resolves outside the home folder: {}", p.display())
            }
            Refusal::TooBroad(p) => write!(f, "refusing a path this broad: {}", p.display()),
            Refusal::Protected(p) => write!(f, "inside a protected location: {}", p.display()),
        }
    }
}

/// Returns the canonical path if `path` may be cleaned.
pub fn validate(path: &Path, home: &Path, protected: &[PathBuf]) -> Result<PathBuf, Refusal> {
    let real = path.canonicalize().map_err(|_| Refusal::NotFound)?;
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let normal = real
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if real == home || normal < 3 {
        return Err(Refusal::TooBroad(real));
    }
    if !real.starts_with(&home) {
        return Err(Refusal::OutsideHome(real));
    }
    for p in protected {
        let p = p.canonicalize().unwrap_or_else(|_| p.clone());
        if real.starts_with(&p) || p.starts_with(&real) {
            return Err(Refusal::Protected(p));
        }
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals() {
        let t = tempfile::tempdir().unwrap();
        // Fake home three levels deep so its children pass the length check.
        let home = t.path().join("Users/me");
        std::fs::create_dir_all(home.join("proj/node_modules")).unwrap();
        std::fs::create_dir_all(home.join("Backup/dev1")).unwrap();
        std::fs::create_dir_all(t.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(t.path().join("elsewhere"), home.join("sneaky")).unwrap();
        let protected = vec![home.join("Backup")];

        assert!(validate(&home.join("proj/node_modules"), &home, &protected).is_ok());
        assert_eq!(
            validate(Path::new("/"), &home, &protected),
            Err(Refusal::TooBroad("/".into()))
        );
        assert!(matches!(
            validate(&home, &home, &protected),
            Err(Refusal::TooBroad(_))
        ));
        assert!(matches!(
            validate(&home.join("sneaky"), &home, &protected),
            Err(Refusal::OutsideHome(_))
        ));
        assert!(matches!(
            validate(&home.join("Backup/dev1"), &home, &protected),
            Err(Refusal::Protected(_))
        ));
        // A parent of a protected location is refused too.
        assert!(matches!(
            validate(&home.join("proj/../Backup"), &home, &protected),
            Err(Refusal::Protected(_))
        ));
        assert_eq!(
            validate(&home.join("nope"), &home, &protected),
            Err(Refusal::NotFound)
        );
        assert!(matches!(
            validate(Path::new("/usr/lib"), &home, &protected),
            Err(Refusal::TooBroad(_))
        ));
        assert!(matches!(
            validate(Path::new("/usr/share/man"), &home, &protected),
            Err(Refusal::OutsideHome(_))
        ));
    }
}
