//! Where aml keeps its own state (config, staged mod library, profile).
//!
//! Two layouts:
//! - **System** (default install): follows the OS — `<config_dir>/aml` +
//!   `<data_dir>/aml`.
//! - **Portable**: everything lives in one `aml-data/` folder next to the
//!   executable, so the whole thing can be dropped on a USB stick or into the
//!   game folder and run with zero install and nothing written to system dirs.
//!
//! Portable mode turns on when any of these is true:
//! - the env var `AML_PORTABLE` is set (to anything non-empty), or
//! - a marker file `aml-portable.txt` sits next to the executable, or
//! - an `aml-data/` directory already exists next to the executable
//!   (so an unzipped portable bundle is portable on first run automatically).

use crate::HostError;
use std::path::PathBuf;

/// Resolved storage locations for this run.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// `config.json` (holds the AES key).
    pub config_file: PathBuf,
    /// Root of the staged mod library (`mods/<id>/…`).
    pub mods_root: PathBuf,
    /// `profile.json` (legacy single-profile enable/priority state; migrated
    /// into `profiles/default.json` on first multi-profile use).
    pub profile_file: PathBuf,
    /// Directory of named profiles (`profiles/<name>.json`).
    pub profiles_dir: PathBuf,
    /// True when running in portable mode.
    pub portable: bool,
}

impl AppPaths {
    /// Resolve the active layout (portable if detected, else system dirs).
    pub fn resolve() -> Result<Self, HostError> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let portable = exe_dir.as_deref().map(portable_here).unwrap_or(false);

        let sys = || -> Result<(PathBuf, PathBuf), HostError> {
            let config = dirs::config_dir()
                .ok_or_else(|| HostError::Other("no config dir on this platform".into()))?
                .join("aml");
            let data = dirs::data_dir()
                .ok_or_else(|| HostError::Other("no data dir on this platform".into()))?
                .join("aml");
            Ok((config, data))
        };

        Ok(if portable {
            // Safe unwrap: `portable` can only be true when exe_dir is Some.
            let base = exe_dir.unwrap().join("aml-data");
            layout(&base, &base, true)
        } else {
            let (config, data) = sys()?;
            layout(&config, &data, false)
        })
    }
}

/// Compose the three storage paths from a config base and a data base.
fn layout(config_base: &std::path::Path, data_base: &std::path::Path, portable: bool) -> AppPaths {
    AppPaths {
        config_file: config_base.join("config.json"),
        mods_root: data_base.join("mods"),
        profile_file: config_base.join("profile.json"),
        profiles_dir: config_base.join("profiles"),
        portable,
    }
}

/// Is portable mode active for a bundle living in `exe_dir`? True if the env var
/// `AML_PORTABLE` is set non-empty, a `aml-portable.txt` marker sits alongside, or
/// an `aml-data/` folder already exists there (unzipped bundle → portable on first run).
fn portable_here(exe_dir: &std::path::Path) -> bool {
    let forced = std::env::var_os("AML_PORTABLE")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    forced || exe_dir.join("aml-portable.txt").is_file() || exe_dir.join("aml-data").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn portable_layout_puts_everything_under_one_base() {
        let base = Path::new("/opt/aml/aml-data");
        let p = layout(base, base, true);
        assert!(p.portable);
        assert_eq!(p.config_file, base.join("config.json"));
        assert_eq!(p.mods_root, base.join("mods"));
        assert_eq!(p.profile_file, base.join("profile.json"));
        assert_eq!(p.profiles_dir, base.join("profiles"));
    }

    #[test]
    fn system_layout_splits_config_and_data() {
        let config = Path::new("/home/u/.config/aml");
        let data = Path::new("/home/u/.local/share/aml");
        let p = layout(config, data, false);
        assert!(!p.portable);
        assert_eq!(p.config_file, config.join("config.json"));
        assert_eq!(p.profile_file, config.join("profile.json"));
        assert_eq!(p.profiles_dir, config.join("profiles"));
        assert_eq!(p.mods_root, data.join("mods"));
    }

    #[test]
    fn marker_file_triggers_portable() {
        let dir = std::env::temp_dir().join("aml-portable-test-marker");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("aml-portable.txt"), b"").unwrap();
        assert!(portable_here(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
