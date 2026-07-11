//! Locate the game install across Steam library folders, on Linux and Windows.
//!
//! Steam scatters games across multiple library folders listed in
//! `steamapps/libraryfolders.vdf`. We do a lightweight scan of that file (no
//! full VDF parser needed — we only want the `"path"` values) and then look for
//! `steamapps/common/Echoes of Aincrad` in each.

use crate::{HostError, APP_ID, GAME_DIR_NAME};
use aml_core::layout::{guess_project_name, InstallLayout};
use std::path::{Path, PathBuf};

/// How the game will actually execute — determines the injection strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runtime {
    /// Native Windows process.
    WindowsNative,
    /// Windows binary under Proton/Wine, with a compatdata prefix.
    Proton { prefix: PathBuf },
}

/// A resolved game install.
#[derive(Debug, Clone)]
pub struct GameInstall {
    pub layout: InstallLayout,
    pub runtime: Runtime,
    /// The Steam library root this install lives under.
    pub library_root: PathBuf,
}

/// Find the game by scanning all Steam library folders on this machine.
pub fn find_game() -> Result<GameInstall, HostError> {
    for lib in steam_library_roots() {
        let common = lib.join("steamapps").join("common").join(GAME_DIR_NAME);
        if common.is_dir() {
            return resolve_install(&lib, &common);
        }
    }
    Err(HostError::GameNotFound)
}

/// Resolve a known game directory into a full install descriptor.
pub fn resolve_install(library_root: &Path, game_root: &Path) -> Result<GameInstall, HostError> {
    let project = guess_project_name(game_root)
        .ok_or_else(|| HostError::ProjectNotFound(game_root.display().to_string()))?;
    let layout = InstallLayout::new(game_root, project);
    let runtime = detect_runtime(library_root);
    Ok(GameInstall {
        layout,
        runtime,
        library_root: library_root.to_path_buf(),
    })
}

/// On Windows we run native; on Linux we look for a Proton compatdata prefix.
fn detect_runtime(library_root: &Path) -> Runtime {
    if cfg!(windows) {
        return Runtime::WindowsNative;
    }
    let prefix = library_root
        .join("steamapps")
        .join("compatdata")
        .join(APP_ID.to_string())
        .join("pfx");
    Runtime::Proton { prefix }
}

/// Candidate Steam library roots, most-likely first.
fn steam_library_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for base in steam_base_dirs() {
        // The base itself is always a library.
        roots.push(base.clone());
        // Plus any extra libraries declared in libraryfolders.vdf.
        let vdf = base.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for path in parse_library_paths(&text) {
                roots.push(PathBuf::from(path));
            }
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

/// Default Steam installation directories per platform.
fn steam_base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        dirs.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        dirs.push(PathBuf::from(r"C:\Program Files\Steam"));
    } else if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/Steam"));
        dirs.push(home.join(".steam/steam"));
        dirs.push(home.join(".steam/root"));
        // Flatpak Steam.
        dirs.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// Extract every `"path"  "<value>"` entry from a libraryfolders.vdf.
/// Deliberately tolerant: we only care about the path strings.
fn parse_library_paths(vdf: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in vdf.lines() {
        let line = line.trim();
        // Lines look like:  "path"		"/mnt/games/SteamLibrary"
        if let Some(rest) = line.strip_prefix("\"path\"") {
            if let Some(val) = extract_first_quoted(rest) {
                out.push(val);
            }
        }
    }
    out
}

/// Grab the first "quoted" substring from a fragment.
fn extract_first_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let rest = &s[start + 1..];
    let end = rest.find('"')?;
    // VDF escapes backslashes as \\ ; unescape for real paths (Windows).
    Some(rest[..end].replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_paths_from_vdf() {
        let vdf = r#"
"libraryfolders"
{
    "0"
    {
        "path"		"/home/junie/.local/share/Steam"
        "label"		""
    }
    "1"
    {
        "path"		"/mnt/games/SteamLibrary"
    }
}
"#;
        let paths = parse_library_paths(vdf);
        assert_eq!(
            paths,
            vec![
                "/home/junie/.local/share/Steam".to_string(),
                "/mnt/games/SteamLibrary".to_string()
            ]
        );
    }

    #[test]
    fn unescapes_windows_paths() {
        let vdf = r#"        "path"		"D:\\SteamLibrary""#;
        let paths = parse_library_paths(vdf);
        assert_eq!(paths, vec!["D:\\SteamLibrary".to_string()]);
    }

    #[test]
    fn ignores_non_path_lines() {
        let vdf = r#"        "label"		"whatever""#;
        assert!(parse_library_paths(vdf).is_empty());
    }
}
