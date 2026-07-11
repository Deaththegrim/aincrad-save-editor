//! Finding the game save + our working directory, cross-platform.

use std::path::PathBuf;

/// The save's path relative to a "LocalAppData" root.
const REL: &str = "EchoesofAincrad/Saved/SaveGames/SaveData.sav";

/// Locate `SaveData.sav`: Windows `%LOCALAPPDATA%`, or the Steam Proton prefix on Linux.
pub fn find_save() -> Option<PathBuf> {
    // Native Windows.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local).join(REL);
        if p.is_file() {
            return Some(p);
        }
    }
    // Linux + Proton: <steam>/steamapps/compatdata/2244210/pfx/drive_c/users/steamuser/AppData/Local/…
    if let Some(home) = dirs_home() {
        let prefix = home
            .join(".local/share/Steam/steamapps/compatdata/2244210/pfx/drive_c/users/steamuser/AppData/Local")
            .join(REL);
        if prefix.is_file() {
            return Some(prefix);
        }
    }
    None
}

/// Where the editor keeps its working copy + extracted thumbnails.
/// Where the editor keeps its data (work copy, thumbnails, looks). Portable —
/// `aml-data/save-editor/` next to the exe — when run from a bundle (matching the
/// AES-key config), else the system data dir. Keeps a downloaded bundle fully
/// self-contained.
fn data_root() -> PathBuf {
    if let Some(base) = portable_base() {
        return base.join("save-editor");
    }
    dirs_data()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("aml")
        .join("save-editor")
}

/// The `aml-data/` folder next to the exe, if portable mode is active (an
/// `aml-data/` dir or `aml-portable.txt` marker sits alongside, or AML_PORTABLE).
fn portable_base() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let forced = std::env::var_os("AML_PORTABLE").map(|v| !v.is_empty()).unwrap_or(false);
    let data = exe_dir.join("aml-data");
    (forced || exe_dir.join("aml-portable.txt").is_file() || data.is_dir()).then_some(data)
}

pub fn work_copy_path() -> PathBuf {
    data_root().join("SaveData.work.sav")
}

pub fn thumbs_dir() -> PathBuf {
    data_root().join("thumbnails")
}

/// Where saved appearance presets ("looks") live.
pub fn looks_dir() -> PathBuf {
    data_root().join("looks")
}

/// Diagnostics log — appended to on notable events so a user can send it if
/// something (e.g. Windows key recovery) doesn't work.
pub fn log_path() -> PathBuf {
    data_root().join("editor.log")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn dirs_data() -> Option<PathBuf> {
    // XDG_DATA_HOME or ~/.local/share on Linux; %APPDATA% on Windows.
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(x));
    }
    if let Some(a) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(a));
    }
    dirs_home().map(|h| h.join(".local/share"))
}
