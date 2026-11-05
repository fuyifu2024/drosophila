//! Portable data discovery and the OS default browser. No browser-specific flags.
use std::path::{Path, PathBuf};
use std::process::Command;

fn choose_data(executable: Option<&Path>, cwd: &Path, source: &Path) -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(parent) = executable.and_then(Path::parent) {
        candidates.push(parent.join("data"));
    }
    candidates.push(cwd.join("data"));
    // Convenience for cargo run from another working directory; portable
    // distributions always use data beside the executable instead.
    candidates.push(source.join("data"));
    candidates.into_iter().find(|p| p.join("brain.bin").is_file())
        .unwrap_or_else(|| cwd.join("data"))
}

pub fn data_dir() -> PathBuf {
    choose_data(std::env::current_exe().ok().as_deref(),
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        Path::new(env!("CARGO_MANIFEST_DIR")))
}

pub fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = result {
        eprintln!("Could not open your browser ({error}). Open {url} manually.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_data_wins_over_the_build_machine_and_working_directory() {
        let root = std::env::temp_dir().join(format!("drosophila-paths-{}", std::process::id()));
        let packaged = root.join("package/data");
        let working = root.join("working/data");
        let source = root.join("source/data");
        for dir in [&packaged, &working, &source] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("brain.bin"), b"fixture").unwrap();
        }
        assert_eq!(choose_data(Some(&root.join("package/drosophila")), &root.join("working"), &root.join("source")), packaged);
        assert_eq!(choose_data(None, &root.join("working"), &root.join("source")), working);
        std::fs::remove_dir_all(root).unwrap();
    }
}
