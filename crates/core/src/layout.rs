//! Pure description of where things live inside a UE5 game install.
//!
//! This is data, not I/O — it computes canonical paths from a game root and the
//! UE project name. The project name (the `<Project>` in
//! `<Project>/Content/Paks`) is game-specific and unknown until the game is
//! installed; see docs/UNKNOWNS.md. Until then callers pass a placeholder.

use std::path::{Path, PathBuf};

/// Standard Unreal Engine shipping layout, rooted at the game install dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLayout {
    /// e.g. `.../steamapps/common/Echoes of Aincrad`
    pub game_root: PathBuf,
    /// The UE project folder name, e.g. `EchoesOfAincrad`. GAME-SPECIFIC.
    pub project: String,
}

impl InstallLayout {
    pub fn new(game_root: impl Into<PathBuf>, project: impl Into<String>) -> Self {
        Self {
            game_root: game_root.into(),
            project: project.into(),
        }
    }

    /// `<root>/<Project>` — the UE project directory.
    pub fn project_dir(&self) -> PathBuf {
        self.game_root.join(&self.project)
    }

    /// `<root>/<Project>/Binaries/Win64` — the shipping exe + UE4SS proxy DLL.
    pub fn win64_dir(&self) -> PathBuf {
        self.project_dir().join("Binaries").join("Win64")
    }

    /// `<root>/<Project>/Content/Paks` — cooked content archives.
    pub fn paks_dir(&self) -> PathBuf {
        self.project_dir().join("Content").join("Paks")
    }

    /// `<root>/<Project>/Content/Paks/~mods` — loose pak mods load from here.
    /// The `~` prefix makes UE mount them after the base game paks.
    pub fn mods_dir(&self) -> PathBuf {
        self.paks_dir().join("~mods")
    }

    /// `<root>/<Project>/Content/Paks/LogicMods` — Blueprint mods, loaded by the
    /// UE4SS BP mod loader. Separate from asset paks in `~mods`.
    pub fn logic_mods_dir(&self) -> PathBuf {
        self.paks_dir().join("LogicMods")
    }

    /// `<win64>/ue4ss` — UE4SS itself (modern layout: everything but the proxy
    /// DLL lives in this subfolder).
    pub fn ue4ss_dir(&self) -> PathBuf {
        self.win64_dir().join("ue4ss")
    }

    /// `<win64>/ue4ss/Mods` — UE4SS Lua / C++ mods.
    pub fn ue4ss_mods_dir(&self) -> PathBuf {
        self.ue4ss_dir().join("Mods")
    }

    /// `<win64>/ue4ss/Mods/mods.txt` — UE4SS load list (name + enabled flag).
    pub fn ue4ss_mods_txt(&self) -> PathBuf {
        self.ue4ss_mods_dir().join("mods.txt")
    }

    /// The proxy DLL UE4SS ships as its injection vector.
    pub fn proxy_dll(&self) -> PathBuf {
        self.win64_dir().join("dwmapi.dll")
    }

    /// Where aml records what it deployed, so the next deploy can remove mods
    /// that were since disabled/removed. Lives in the install so it tracks the
    /// actual on-disk state, not per-user config.
    pub fn deploy_manifest(&self) -> PathBuf {
        self.game_root.join(".aml-deploy.json")
    }

    /// True if this looks like a real install (project dir + paks present).
    /// The only method here that touches the filesystem — used for validation.
    pub fn looks_installed(&self) -> bool {
        self.paks_dir().is_dir()
    }
}

/// Best-effort guess of the UE project folder name by scanning a game root.
/// Returns the first child of `game_root` that contains `Content/Paks`.
/// Pure-ish: reads directory entries but mutates nothing.
pub fn guess_project_name(game_root: &Path) -> Option<String> {
    let entries = std::fs::read_dir(game_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("Content").join("Paks").is_dir() {
            return path.file_name()?.to_str().map(|s| s.to_string());
        }
    }
    None
}
