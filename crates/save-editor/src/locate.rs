//! Finding the game save + our working directory, cross-platform.

use std::path::{Path, PathBuf};

/// The save's path relative to a "LocalAppData" root.
const REL: &str = "EchoesofAincrad/Saved/SaveGames/SaveData.sav";

/// Locate `SaveData.sav`: Windows `%LOCALAPPDATA%`, or the Steam Proton prefix on
/// Linux. On Linux we ask `aml-host` to find the install (which handles multiple
/// Steam libraries, other drives, and Flatpak) and look inside its Proton prefix;
/// we also try the common Steam roots directly. The prefix user is globbed, not
/// assumed to be `steamuser`. Users can always fall back to "Open save…".
pub fn find_save() -> Option<PathBuf> {
    // Native Windows.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local).join(REL);
        if p.is_file() {
            return Some(p);
        }
    }
    // Linux/Proton: use the game's detected Proton prefix (robust to library/drive).
    if let Ok(game) = aml_host::find_game() {
        if let aml_host::Runtime::Proton { prefix } = &game.runtime {
            if let Some(p) = find_in_users(&prefix.join("drive_c/users")) {
                return Some(p);
            }
        }
    }
    // Fallback: probe the common Steam roots' compatdata directly.
    for base in linux_steam_bases() {
        let users = base.join("steamapps/compatdata/2244210/pfx/drive_c/users");
        if let Some(p) = find_in_users(&users) {
            return Some(p);
        }
    }
    None
}

/// Look for `<users>/<any-user>/AppData/Local/<REL>` — the prefix user is often
/// `steamuser` but isn't guaranteed, so we scan every user folder.
fn find_in_users(users_dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(users_dir).ok()?.flatten() {
        let p = entry.path().join("AppData/Local").join(REL);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Common Steam install roots on Linux (mirrors aml-host's list), incl. Flatpak.
fn linux_steam_bases() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = dirs_home() {
        v.push(h.join(".local/share/Steam"));
        v.push(h.join(".steam/steam"));
        v.push(h.join(".steam/root"));
        v.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    v
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

/// Where the working copy's timestamped backups land (see `aml_save::backup`).
/// The LIVE save's backups live next to the game's save file instead.
pub fn work_backups_dir() -> PathBuf {
    data_root().join("backups")
}

/// Where the bundled voice-preview clips live (`voices/<lang>/<Voice>_<n>.ogg`).
pub fn voices_dir() -> PathBuf {
    data_root().join("voices")
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
