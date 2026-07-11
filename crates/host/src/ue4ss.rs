//! Stage UE4SS into a game install from a downloaded release zip.
//!
//! UE4SS ≥4.0 zips extract directly into `Binaries/Win64/`, yielding the proxy
//! `dwmapi.dll` beside the shipping exe and a `ue4ss/` subfolder (`UE4SS.dll`,
//! `UE4SS-settings.ini`, `Mods/`). We extract there and verify the result.
//!
//! We do NOT vendor UE4SS — the user downloads the release matching the game's
//! confirmed UE version (avoid any `zDEV` asset). See docs/UNKNOWNS.md.

use crate::HostError;
use aml_core::layout::InstallLayout;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Whether UE4SS is present in an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ue4ssStatus {
    /// Proxy DLL + `ue4ss/UE4SS.dll` both present.
    Installed,
    /// Proxy DLL present but the `ue4ss/` payload is missing.
    ProxyOnly,
    /// Nothing staged.
    NotInstalled,
}

/// Live UE4SS runtime signals read off disk — what the GUI shows so the user can
/// confirm UE4SS actually loaded and their in-game dumps landed.
#[derive(Debug, Default, Clone)]
pub struct Diagnostics {
    /// Log shows mods started and no fatal AOB-scan timeout.
    pub loaded: bool,
    /// Last N lines of UE4SS.log.
    pub log_tail: Vec<String>,
    /// Count of generated CXX SDK headers (Ctrl+H).
    pub sdk_headers: usize,
    /// An object dump (Ctrl+J) is present.
    pub has_object_dump: bool,
    /// A `.usmap` mappings file is present.
    pub has_usmap: bool,
}

/// Read UE4SS diagnostics from the install (log tail + dump artifacts).
pub fn diagnostics(layout: &InstallLayout, tail_lines: usize) -> Diagnostics {
    let mut d = Diagnostics::default();
    let dir = layout.ue4ss_dir();

    if let Ok(text) = std::fs::read_to_string(dir.join("UE4SS.log")) {
        d.loaded = text.contains("Starting Lua mod") && !text.contains("PS scan timed out");
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(tail_lines);
        d.log_tail = lines[start..].iter().map(|s| s.to_string()).collect();
    }

    if let Ok(rd) = std::fs::read_dir(dir.join("CXXHeaderDump")) {
        d.sdk_headers = rd
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "hpp").unwrap_or(false))
            .count();
    }

    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.contains("objectdump") {
                d.has_object_dump = true;
            }
            if name.ends_with(".usmap") {
                d.has_usmap = true;
            }
        }
    }

    d
}

/// Inspect an install for UE4SS.
pub fn status(layout: &InstallLayout) -> Ue4ssStatus {
    let proxy = layout.proxy_dll().is_file();
    let core = layout.ue4ss_dir().join("UE4SS.dll").is_file();
    match (proxy, core) {
        (true, true) => Ue4ssStatus::Installed,
        (true, false) => Ue4ssStatus::ProxyOnly,
        _ => Ue4ssStatus::NotInstalled,
    }
}

/// Extract a UE4SS release zip into `Binaries/Win64`. Returns the list of files
/// written (relative to Win64). With `dry_run`, lists without writing.
pub fn stage_from_zip(
    zip_path: &Path,
    layout: &InstallLayout,
    dry_run: bool,
) -> Result<Vec<String>, HostError> {
    let win64 = layout.win64_dir();
    if !dry_run && !win64.is_dir() {
        return Err(HostError::Other(format!(
            "target dir does not exist: {} (is the game installed?)",
            win64.display()
        )));
    }

    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HostError::Other(format!("bad zip {}: {e}", zip_path.display())))?;

    let mut written = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| HostError::Other(format!("zip entry {i}: {e}")))?;

        let Some(rel) = safe_relative_path(entry.name()) else {
            return Err(HostError::Other(format!(
                "refusing unsafe zip path: {}",
                entry.name()
            )));
        };

        // Skip developer-only builds if they somehow appear in the archive.
        if rel.to_string_lossy().contains("zDEV") {
            continue;
        }

        let dest = win64.join(&rel);
        if entry.is_dir() {
            if !dry_run {
                std::fs::create_dir_all(&dest)?;
            }
            continue;
        }

        written.push(rel.to_string_lossy().to_string());
        if dry_run {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        std::fs::write(&dest, &buf)?;
    }

    // Sanity check: the archive should have contained the proxy + core dll.
    let has_proxy = written.iter().any(|w| w.eq_ignore_ascii_case("dwmapi.dll"));
    let has_core = written
        .iter()
        .any(|w| w.to_lowercase().ends_with("ue4ss.dll"));
    if !has_proxy || !has_core {
        return Err(HostError::Other(format!(
            "zip didn't look like a UE4SS release (proxy dll: {has_proxy}, UE4SS.dll: {has_core}); \
             did you pick the standard UE4SS_vX.X.X zip and not a zDEV build?"
        )));
    }

    Ok(written)
}

/// Reject absolute paths and `..` traversal in zip entry names.
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            // Absolute roots, prefixes, or `..` are all rejected.
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_paths() {
        assert!(safe_relative_path("../evil.dll").is_none());
        assert!(safe_relative_path("/etc/passwd").is_none());
        assert!(safe_relative_path("ue4ss/../../x").is_none());
    }

    #[test]
    fn accepts_normal_paths() {
        assert_eq!(
            safe_relative_path("ue4ss/Mods/mods.txt"),
            Some(PathBuf::from("ue4ss/Mods/mods.txt"))
        );
        assert_eq!(
            safe_relative_path("dwmapi.dll"),
            Some(PathBuf::from("dwmapi.dll"))
        );
    }

    #[test]
    fn status_reports_not_installed_on_empty() {
        let layout = InstallLayout::new("/nonexistent/game", "Proj");
        assert_eq!(status(&layout), Ue4ssStatus::NotInstalled);
    }
}
