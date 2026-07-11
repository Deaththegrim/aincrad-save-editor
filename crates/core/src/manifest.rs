//! Mod manifest schema and profile model.
//!
//! A *mod* is described by a `mod.json` manifest that ships inside the mod's
//! folder. A *profile* is the user's ordered, enabled/disabled set of mods for
//! one game install. Both are plain serde types — no I/O here.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A stable, unique identifier for a mod, e.g. "author.better-swords".
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModId(pub String);

impl fmt::Display for ModId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ModId {
    fn from(s: &str) -> Self {
        ModId(s.to_string())
    }
}

/// How a mod is loaded into the game. Mirrors the UE5 modding surface as
/// confirmed against the Echoes of Aincrad demo modding scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModKind {
    /// An asset/UI `.pak` (or IoStore) archive → `Content/Paks/~mods`.
    Pak,
    /// A Blueprint "LogicMod" `.pak` → `Content/Paks/LogicMods` (loaded by the
    /// UE4SS BP mod loader). A distinct folder from asset paks.
    Logic,
    /// A UE4SS Lua mod (folder with `scripts/main.lua`) → `ue4ss/Mods`.
    Lua,
    /// A UE4SS C++ mod (compiled `.dll`) → `ue4ss/Mods`.
    Cpp,
}

impl ModKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ModKind::Pak => "pak",
            ModKind::Logic => "logic",
            ModKind::Lua => "lua",
            ModKind::Cpp => "cpp",
        }
    }
}

/// The manifest that ships with a single mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModManifest {
    pub id: ModId,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub kind: ModKind,
    /// Lower numbers load earlier. Ties broken by id for determinism.
    #[serde(default)]
    pub priority: i32,
    /// Other mod ids that must load before this one.
    #[serde(default)]
    pub requires: Vec<ModId>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_version() -> String {
    "0.0.0".to_string()
}

/// One enabled/disabled entry in a user's profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileEntry {
    #[serde(flatten)]
    pub manifest: ModManifest,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// User override for load priority; falls back to the manifest's.
    #[serde(default)]
    pub priority_override: Option<i32>,
}

fn default_true() -> bool {
    true
}

impl ProfileEntry {
    pub fn effective_priority(&self) -> i32 {
        self.priority_override.unwrap_or(self.manifest.priority)
    }
}

/// A user's full mod set for one game install.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default = "default_profile_name")]
    pub name: String,
    #[serde(default)]
    pub mods: Vec<ProfileEntry>,
}

fn default_profile_name() -> String {
    "default".to_string()
}

impl Profile {
    /// Enabled entries only.
    pub fn enabled(&self) -> impl Iterator<Item = &ProfileEntry> {
        self.mods.iter().filter(|m| m.enabled)
    }
}
