// Embed every catalog/*.toml into the binary (see src/catalog.rs).
use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("catalog");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let mut out = String::from("pub static EMBEDDED: &[(&str, &str)] = &[\n");
    for f in files {
        println!("cargo:rerun-if-changed={}", f.display());
        out += &format!(
            "    ({:?}, include_str!({:?})),\n",
            f.file_name().unwrap().to_string_lossy(),
            f.display().to_string()
        );
    }
    out += "];\n";
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("catalog_files.rs"),
        out,
    )
    .unwrap();
}
