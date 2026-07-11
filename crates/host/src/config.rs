//! Persistent app config — currently the pak AES key, so content extraction and
//! packing "just work" without pasting the key on every command. The path is
//! resolved by [`crate::paths::AppPaths`] (`<config_dir>/aml/config.json` when
//! installed, or `aml-data/config.json` beside the exe in portable mode). The key
//! is a secret (DRM material): it lives in the user's private config, never in the repo.

use crate::HostError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Pak AES key (hex `0x…` or base64), if the user has set it.
    #[serde(default)]
    pub aes_key: Option<String>,
    /// UI language code (e.g. "en", "ja"), for the save editor.
    #[serde(default)]
    pub lang: Option<String>,
    /// Active mod profile name; `None` means the implicit "default" profile.
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Nexus Mods personal API key, for resolving `nxm://` download links.
    #[serde(default)]
    pub nexus_api_key: Option<String>,
}

impl AppConfig {
    pub fn path() -> Result<PathBuf, HostError> {
        Ok(crate::paths::AppPaths::resolve()?.config_file)
    }

    /// Load config (empty default if none / unreadable).
    pub fn load() -> Self {
        Self::path()
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), HostError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| HostError::Other(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }
}
