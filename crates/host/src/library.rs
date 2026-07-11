//! The mod library: staging area + persisted enable/disable & priority state.
//!
//! Each mod is a folder under `<data_dir>/aml/mods/<id>/` containing a
//! `mod.json` and its payload (a `.pak`, or a UE4SS Lua/C++ mod folder). The
//! user's per-mod choices (enabled, priority override) live separately in
//! `<config_dir>/aml/profile.json` so re-scanning the library never clobbers them.

use crate::HostError;
use aml_core::manifest::{ModId, ModKind, ModManifest, Profile, ProfileEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// On-disk staging area for mods.
#[derive(Debug, Clone)]
pub struct ModLibrary {
    pub root: PathBuf,
    /// Where per-mod enable/priority state is persisted.
    pub state_path: PathBuf,
}

/// Persisted user choices, overlaid on the scanned library.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileState {
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
    #[serde(default)]
    pub priority: BTreeMap<String, i32>,
}

impl ModLibrary {
    /// Default location, resolved by [`crate::paths::AppPaths`]:
    /// `<data_dir>/aml/mods` + `<config_dir>/aml/profile.json` when installed, or
    /// `aml-data/{mods,profile.json}` beside the exe in portable mode.
    pub fn default_location() -> Result<Self, HostError> {
        let paths = crate::paths::AppPaths::resolve()?;
        // The active named profile supplies the enable/priority overlay; this
        // also migrates a legacy profile.json into profiles/default.json.
        let state_path = crate::profiles::Profiles::resolve()?.active_file();
        Ok(Self {
            root: paths.mods_root,
            state_path,
        })
    }

    pub fn mod_dir(&self, id: &ModId) -> PathBuf {
        self.root.join(&id.0)
    }

    /// The first `.pak` staged inside a mod's folder.
    pub fn find_pak(&self, id: &ModId) -> Option<PathBuf> {
        let dir = self.mod_dir(id);
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().map(|e| e.eq_ignore_ascii_case("pak")).unwrap_or(false))
    }

    // --- persisted state -----------------------------------------------------

    pub fn load_state(&self) -> Result<ProfileState, HostError> {
        match std::fs::read_to_string(&self.state_path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| HostError::Other(format!("{}: {e}", self.state_path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ProfileState::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save_state(&self, state: &ProfileState) -> Result<(), HostError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(state)
            .map_err(|e| HostError::Other(e.to_string()))?;
        std::fs::write(&self.state_path, text)?;
        Ok(())
    }

    /// Set (or clear) the enabled flag for a mod and persist it.
    pub fn set_enabled(&self, id: &ModId, enabled: bool) -> Result<(), HostError> {
        let mut state = self.load_state()?;
        state.enabled.insert(id.0.clone(), enabled);
        self.save_state(&state)
    }

    /// Set (or clear, via None) a priority override for a mod and persist it.
    pub fn set_priority(&self, id: &ModId, priority: Option<i32>) -> Result<(), HostError> {
        let mut state = self.load_state()?;
        match priority {
            Some(p) => {
                state.priority.insert(id.0.clone(), p);
            }
            None => {
                state.priority.remove(&id.0);
            }
        }
        self.save_state(&state)
    }

    // --- profile assembly ----------------------------------------------------

    /// Build a Profile by scanning `mod.json`s and overlaying persisted state.
    pub fn load_profile(&self) -> Result<Profile, HostError> {
        let state = self.load_state()?;
        let mut mods = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let manifest_path = entry.path().join("mod.json");
                if !manifest_path.is_file() {
                    continue;
                }
                let text = std::fs::read_to_string(&manifest_path)?;
                let manifest: ModManifest = serde_json::from_str(&text)
                    .map_err(|e| HostError::Other(format!("{}: {e}", manifest_path.display())))?;
                let id = manifest.id.0.clone();
                mods.push(ProfileEntry {
                    enabled: state.enabled.get(&id).copied().unwrap_or(true),
                    priority_override: state.priority.get(&id).copied(),
                    manifest,
                });
            }
        }
        mods.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        Ok(Profile {
            name: "library".into(),
            mods,
        })
    }

    // --- ingestion -----------------------------------------------------------

    /// Import a mod from a `.pak` file, a `.zip`, or a mod folder into the
    /// library. Returns the assigned id. `id_override` names it; otherwise the
    /// id is derived from the source's file/dir stem.
    pub fn add_mod(
        &self,
        source: &Path,
        id_override: Option<&str>,
    ) -> Result<ModId, HostError> {
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mod");
        let cleaned = sanitize_id(id_override.unwrap_or(stem));
        if cleaned.is_empty() {
            return Err(HostError::Other(format!(
                "cannot derive a mod id from '{}'; pass --id",
                id_override.unwrap_or(stem)
            )));
        }
        let id = ModId(cleaned);
        let dest = self.mod_dir(&id);
        if dest.exists() {
            return Err(HostError::Other(format!(
                "mod '{id}' already in library; remove it first"
            )));
        }
        std::fs::create_dir_all(&dest)?;

        if source.is_dir() {
            copy_dir_recursive(source, &dest, &[])?;
        } else if source.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) {
            extract_zip_into(source, &dest)?;
        } else if source.extension().map(|e| e.eq_ignore_ascii_case("pak")).unwrap_or(false) {
            let name = source.file_name().unwrap();
            std::fs::copy(source, dest.join(name))?;
        } else {
            std::fs::remove_dir_all(&dest).ok();
            return Err(HostError::Other(format!(
                "unsupported mod source '{}': expected a .pak, .zip, or mod folder",
                source.display()
            )));
        }

        // Write a manifest if the mod didn't ship one.
        let manifest_path = dest.join("mod.json");
        if !manifest_path.is_file() {
            let kind = detect_kind(&dest);
            let manifest = ModManifest {
                id: id.clone(),
                name: id.0.clone(),
                version: "0.0.0".into(),
                kind,
                priority: 0,
                requires: vec![],
                author: None,
                description: None,
            };
            let text = serde_json::to_string_pretty(&manifest)
                .map_err(|e| HostError::Other(e.to_string()))?;
            std::fs::write(&manifest_path, text)?;
        }
        Ok(id)
    }

    /// Remove a mod folder from the library (and its persisted state).
    pub fn remove_mod(&self, id: &ModId) -> Result<(), HostError> {
        let dir = self.mod_dir(id);
        if !dir.is_dir() {
            return Err(HostError::Other(format!("mod '{id}' not in library")));
        }
        std::fs::remove_dir_all(&dir)?;
        let mut state = self.load_state()?;
        state.enabled.remove(&id.0);
        state.priority.remove(&id.0);
        self.save_state(&state)?;
        Ok(())
    }
}

/// Guess a mod's kind from its staged contents.
fn detect_kind(dir: &Path) -> ModKind {
    let has = |p: &str| dir.join(p).exists();
    if has("scripts/main.lua") || has("Scripts/main.lua") {
        ModKind::Lua
    } else if has("dlls/main.dll") || has("Dlls/main.dll") {
        ModKind::Cpp
    } else {
        // Default: a loose pak mod.
        ModKind::Pak
    }
}

fn sanitize_id(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    cleaned.trim_matches('_').to_string()
}

/// Recursively copy `src` into `dst`, skipping top-level entries in `skip`.
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| *s == name.to_string_lossy()) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to, &[])?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn extract_zip_into(zip_path: &Path, dest: &Path) -> Result<(), HostError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| HostError::Other(format!("bad zip {}: {e}", zip_path.display())))?;
    archive
        .extract(dest)
        .map_err(|e| HostError::Other(format!("extract {}: {e}", zip_path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_ids() {
        assert_eq!(sanitize_id("Author's Cool Mod!"), "Author_s_Cool_Mod");
        assert_eq!(sanitize_id("author.mod-1_v2"), "author.mod-1_v2");
    }

    #[test]
    fn detect_kind_from_contents() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("scripts")).unwrap();
        std::fs::write(tmp.path().join("scripts/main.lua"), "print('hi')").unwrap();
        assert_eq!(detect_kind(tmp.path()), ModKind::Lua);
    }
}
