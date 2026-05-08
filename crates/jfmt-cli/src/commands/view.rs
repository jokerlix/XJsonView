use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

const BINARY_NAME: &str = if cfg!(windows) {
    "jfmt-viewer.exe"
} else {
    "jfmt-viewer"
};

pub fn run<P: AsRef<Path>>(file: P) -> Result<()> {
    let file = file.as_ref();
    if !file.exists() {
        return Err(anyhow!(
            "file not found: {} — did you mean a different path?",
            file.display()
        ));
    }
    let abs =
        std::fs::canonicalize(file).with_context(|| format!("canonicalize {}", file.display()))?;

    let viewer = locate_viewer().ok_or_else(|| {
        anyhow!(
            "could not find {BINARY_NAME} on PATH or next to jfmt — install the GUI \
             from https://github.com/jokerlix/XJsonView/releases"
        )
    })?;

    let status = std::process::Command::new(&viewer)
        .arg(&abs)
        .status()
        .with_context(|| format!("spawn {}", viewer.display()))?;
    if !status.success() {
        return Err(anyhow!(
            "{} exited with status {}",
            viewer.display(),
            status
        ));
    }
    Ok(())
}

/// Search order:
/// 1. Same directory as the current `jfmt` executable.
/// 2. PATH lookup.
/// 3. macOS only: /Applications/jfmt-viewer.app/Contents/MacOS/jfmt-viewer.
fn locate_viewer() -> Option<PathBuf> {
    if let Ok(jfmt_self) = std::env::current_exe() {
        if let Some(dir) = jfmt_self.parent() {
            let candidate = dir.join(BINARY_NAME);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    if let Ok(path) = which(BINARY_NAME) {
        return Some(path);
    }
    #[cfg(target_os = "macos")]
    {
        let app = PathBuf::from("/Applications/jfmt-viewer.app/Contents/MacOS/jfmt-viewer");
        if app.exists() {
            return Some(app);
        }
    }
    None
}

fn which(name: &str) -> std::io::Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "PATH not set"))?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        name.to_string(),
    ))
}
