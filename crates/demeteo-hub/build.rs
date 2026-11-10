use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

/// The desktop's bundled starters, the only Workflows a Hub offers. A build
/// without `src-tauri/workflows/` beside this crate fails here, which is why
/// `deploy/hub/Dockerfile` copies `src-tauri/`.
const STARTERS_DIR: &str = "../../src-tauri/workflows";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed={STARTERS_DIR}");

    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR")?).join(STARTERS_DIR);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    files.sort();

    let mut source = String::from("pub(crate) const STARTER_FILES: &[&str] = &[\n");
    for file in files {
        let absolute = fs::canonicalize(&file)?;
        let path = absolute.to_str().ok_or("a starter path is not UTF-8")?;
        writeln!(source, "    include_str!({path:?}),")?;
    }
    source.push_str("];\n");
    fs::write(Path::new(&env::var("OUT_DIR")?).join("starters.rs"), source)?;
    Ok(())
}
