//! Marker files that identify a project directory (PLAN.md §3.1). The walker
//! records them as a bitset on each `DirNode`, so the detector needs no second pass.

use crate::backend::Kind;

pub const PACKAGE_JSON: u32 = 1 << 0;
pub const CARGO_TOML: u32 = 1 << 1;
pub const PODFILE: u32 = 1 << 2;
pub const PACKAGE_SWIFT: u32 = 1 << 3;
pub const POM_XML: u32 = 1 << 4;
/// build.gradle(.kts) or settings.gradle(.kts)
pub const GRADLE: u32 = 1 << 5;
pub const PUBSPEC: u32 = 1 << 6;
/// pyproject.toml, requirements.txt, setup.py, setup.cfg, Pipfile
pub const PYTHON_PROJECT: u32 = 1 << 7;
/// any *.py file
pub const PY_FILE: u32 = 1 << 8;
pub const CMAKE: u32 = 1 << 9;
/// *.csproj / *.fsproj / *.vbproj
pub const DOTNET: u32 = 1 << 10;
/// `.git` directory: a repository root.
pub const GIT_DIR: u32 = 1 << 11;
/// `.git` file: a worktree or submodule (contains `gitdir: ...`).
pub const GIT_FILE: u32 = 1 << 12;
/// CACHEDIR.TAG (cargo target/, many caches): standard "this is a cache" marker.
pub const CACHEDIR_TAG: u32 = 1 << 13;

pub fn marker_for(name: &[u8], kind: Kind) -> u32 {
    if kind == Kind::Dir {
        return if name == b".git" { GIT_DIR } else { 0 };
    }
    match name {
        b"package.json" => PACKAGE_JSON,
        b"Cargo.toml" => CARGO_TOML,
        b"Podfile" => PODFILE,
        b"Package.swift" => PACKAGE_SWIFT,
        b"pom.xml" => POM_XML,
        b"build.gradle" | b"build.gradle.kts" | b"settings.gradle" | b"settings.gradle.kts" => {
            GRADLE
        }
        b"pubspec.yaml" => PUBSPEC,
        b"pyproject.toml" | b"requirements.txt" | b"setup.py" | b"setup.cfg" | b"Pipfile" => {
            PYTHON_PROJECT
        }
        b"CMakeLists.txt" => CMAKE,
        b"CACHEDIR.TAG" => CACHEDIR_TAG,
        b".git" => GIT_FILE,
        n if n.ends_with(b".py") => PY_FILE,
        n if n.ends_with(b".csproj") || n.ends_with(b".fsproj") || n.ends_with(b".vbproj") => {
            DOTNET
        }
        _ => 0,
    }
}
