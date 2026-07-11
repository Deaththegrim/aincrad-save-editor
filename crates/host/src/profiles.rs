//! Named mod profiles.
//!
//! A *profile* is a [`ProfileState`](crate::library::ProfileState) — the
//! enable/disable + priority overlay — stored as `profiles/<name>.json`. Every
//! profile shares the one staged mod library; switching profiles just swaps
//! which overlay is active, so no mod files move. The active profile name lives
//! in [`AppConfig`], and `None` means the implicit `default` profile (which need
//! not have a file on disk until it's first written).

use crate::config::AppConfig;
use crate::paths::AppPaths;
use crate::HostError;
use std::path::PathBuf;

/// The profile that's active when the user has never switched.
pub const DEFAULT_PROFILE: &str = "default";

/// Named-profile storage rooted at `<config>/aml/profiles`.
#[derive(Debug, Clone)]
pub struct Profiles {
    pub dir: PathBuf,
}

impl Profiles {
    /// Resolve the profiles dir from [`AppPaths`], migrating a legacy single
    /// `profile.json` into `profiles/default.json` the first time.
    pub fn resolve() -> Result<Self, HostError> {
        let paths = AppPaths::resolve()?;
        let profiles = Self { dir: paths.profiles_dir.clone() };
        profiles.migrate_legacy(&paths.profile_file)?;
        Ok(profiles)
    }

    /// Path to a profile's state file.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    fn exists(&self, name: &str) -> bool {
        name == DEFAULT_PROFILE || self.file(name).is_file()
    }

    /// Move a pre-multi-profile `profile.json` into `profiles/default.json` once,
    /// so existing users keep their enable/priority choices.
    fn migrate_legacy(&self, legacy: &std::path::Path) -> Result<(), HostError> {
        let default = self.file(DEFAULT_PROFILE);
        if legacy.is_file() && !default.exists() {
            std::fs::create_dir_all(&self.dir)?;
            std::fs::rename(legacy, &default)?;
        }
        Ok(())
    }

    /// All profile names, always including `default`, sorted.
    pub fn list(&self) -> Vec<String> {
        let mut names = vec![DEFAULT_PROFILE.to_string()];
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if stem != DEFAULT_PROFILE {
                            names.push(stem.to_string());
                        }
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// The active profile name (from config; `default` if unset).
    pub fn active(&self) -> String {
        AppConfig::load()
            .active_profile
            .unwrap_or_else(|| DEFAULT_PROFILE.to_string())
    }

    /// State file for the active profile.
    pub fn active_file(&self) -> PathBuf {
        self.file(&self.active())
    }

    /// Create a new empty profile and make it active. Errors if the name is
    /// invalid or already taken.
    pub fn create(&self, name: &str) -> Result<(), HostError> {
        validate_name(name)?;
        if self.exists(name) {
            return Err(HostError::Other(format!("profile '{name}' already exists")));
        }
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.file(name), "{}\n")?;
        self.set_active(name)
    }

    /// Switch the active profile. Errors if it doesn't exist (except `default`,
    /// which is always available even before its file is written).
    pub fn switch(&self, name: &str) -> Result<(), HostError> {
        validate_name(name)?;
        if !self.exists(name) {
            return Err(HostError::Other(format!(
                "profile '{name}' does not exist (create it with `aml profile new {name}`)"
            )));
        }
        self.set_active(name)
    }

    /// Delete a profile's state file. Refuses to delete `default` or the
    /// currently active profile (switch away first).
    pub fn delete(&self, name: &str) -> Result<(), HostError> {
        if name == DEFAULT_PROFILE {
            return Err(HostError::Other("cannot delete the default profile".into()));
        }
        if self.active() == name {
            return Err(HostError::Other(format!(
                "profile '{name}' is active; switch away before deleting it"
            )));
        }
        let file = self.file(name);
        if !file.is_file() {
            return Err(HostError::Other(format!("profile '{name}' does not exist")));
        }
        std::fs::remove_file(file)?;
        Ok(())
    }

    fn set_active(&self, name: &str) -> Result<(), HostError> {
        let mut cfg = AppConfig::load();
        cfg.active_profile = Some(name.to_string());
        cfg.save()
    }
}

/// A profile name must be a non-empty run of `[A-Za-z0-9_-]` so it's a safe file
/// stem on every platform.
pub fn validate_name(name: &str) -> Result<(), HostError> {
    if name.is_empty() {
        return Err(HostError::Other("profile name cannot be empty".into()));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(HostError::Other(format!(
            "invalid profile name '{name}': use only letters, digits, '-' and '_'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles() -> (tempfile::TempDir, Profiles) {
        let tmp = tempfile::tempdir().unwrap();
        let p = Profiles { dir: tmp.path().join("profiles") };
        (tmp, p)
    }

    #[test]
    fn list_always_includes_default() {
        let (_t, p) = profiles();
        assert_eq!(p.list(), vec!["default".to_string()]);
    }

    #[test]
    fn list_picks_up_created_files_sorted() {
        let (_t, p) = profiles();
        std::fs::create_dir_all(&p.dir).unwrap();
        std::fs::write(p.file("zeta"), "{}").unwrap();
        std::fs::write(p.file("alpha"), "{}").unwrap();
        assert_eq!(p.list(), vec!["alpha", "default", "zeta"]);
    }

    #[test]
    fn validate_rejects_bad_names() {
        assert!(validate_name("").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("dots.bad").is_err());
        assert!(validate_name("slash/bad").is_err());
        assert!(validate_name("ok-name_1").is_ok());
    }

    #[test]
    fn delete_refuses_default() {
        let (_t, p) = profiles();
        assert!(p.delete("default").is_err());
    }

    #[test]
    fn delete_missing_errors() {
        let (_t, p) = profiles();
        assert!(p.delete("ghost").is_err());
    }

    #[test]
    fn migrate_moves_legacy_into_default() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("profile.json");
        std::fs::write(&legacy, r#"{"enabled":{"a":false}}"#).unwrap();
        let p = Profiles { dir: tmp.path().join("profiles") };
        p.migrate_legacy(&legacy).unwrap();
        assert!(!legacy.exists());
        let moved = std::fs::read_to_string(p.file("default")).unwrap();
        assert!(moved.contains("\"a\""));
    }

    #[test]
    fn migrate_does_not_clobber_existing_default() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("profile.json");
        std::fs::write(&legacy, r#"{"enabled":{"legacy":true}}"#).unwrap();
        let p = Profiles { dir: tmp.path().join("profiles") };
        std::fs::create_dir_all(&p.dir).unwrap();
        std::fs::write(p.file("default"), r#"{"enabled":{"kept":true}}"#).unwrap();
        p.migrate_legacy(&legacy).unwrap();
        // Existing default is untouched; legacy stays put (not silently merged).
        let def = std::fs::read_to_string(p.file("default")).unwrap();
        assert!(def.contains("kept"));
        assert!(legacy.exists());
    }
}
