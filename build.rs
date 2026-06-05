//! Build script: re-invokes `scripts/build_themes.py` whenever any file in
//! `themes/` is newer than the generated `src/ui/_generated_bundled_themes.rs`.
//!
//! This keeps the generated Rust source in sync with the on-disk themes
//! without requiring the developer to remember to run the script. If
//! Python is not available, we silently skip — the committed generated
//! file is the source of truth and the build proceeds.

use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let themes_dir = Path::new("themes");
    let out_path = Path::new("src/ui/_generated_bundled_themes.rs");
    let script_path = Path::new("scripts/build_themes.py");

    // Always re-run the build when these change. This tells Cargo to
    // invalidate the build cache.
    println!("cargo:rerun-if-changed=themes/");
    println!("cargo:rerun-if-changed=scripts/build_themes.py");
    println!("cargo:rerun-if-changed={}", out_path.display());

    // Only regenerate if themes/ is actually newer than the generated file.
    if !needs_regeneration(themes_dir, out_path) {
        return;
    }

    // Make sure Python is available; fall back to the committed file if not.
    let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());
    let status = Command::new(&python).arg(script_path).status();

    match status {
        Ok(s) if s.success() => {
            // Re-run the build so Cargo notices the new mtime.
            println!("cargo:rerun-if-changed=src/ui/_generated_bundled_themes.rs");
        }
        Ok(s) => {
            println!(
                "cargo:warning=themebuild exited with status {s}; using committed themes file"
            );
        }
        Err(e) => {
            println!("cargo:warning=could not invoke {python} ({e}); using committed themes file");
        }
    }
}

fn needs_regeneration(themes_dir: &Path, out_path: &Path) -> bool {
    if !out_path.exists() {
        return true;
    }
    let Ok(out_meta) = fs::metadata(out_path) else {
        return true;
    };
    let Ok(out_mtime) = out_meta.modified() else {
        return true;
    };
    let Ok(entries) = fs::read_dir(themes_dir) else {
        return false; // no themes dir → nothing to do
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path)
            && let Ok(mtime) = meta.modified()
            && mtime > out_mtime
        {
            return true;
        }
    }
    false
}
