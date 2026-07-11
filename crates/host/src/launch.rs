//! Launch orchestration.
//!
//! Two ways to start a modded session:
//! - **Via Steam** (`steam://rungameid/<appid>`): honours the user's launch
//!   options (including the WINEDLLOVERRIDES we recommend for Proton). Simplest
//!   and most robust; the game's own EAC-free single-player build means there's
//!   nothing to fight.
//! - **Direct**: run the shipping exe (native) or through Proton (Linux) with
//!   our injection env applied. Needs the exe name, which is game-specific and
//!   confirmed once installed (see docs/UNKNOWNS.md).
//!
//! Offline note: this game is confirmed single-player, so there is no anti-cheat
//! to disable. "Offline" here just means we don't require network — we never
//! touch save integrity or online services.

use crate::detect::{GameInstall, Runtime};
use crate::inject::InjectionStrategy;
use crate::{HostError, APP_ID};
use std::path::PathBuf;
use std::process::Command;

/// A fully-described launch command — inspectable before we run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub description: String,
}

impl LaunchPlan {
    /// Launch through the Steam client. The user's per-game launch options
    /// (set once via `aml doctor`) carry the Proton DLL override.
    pub fn via_steam() -> Self {
        LaunchPlan {
            program: if cfg!(windows) { "cmd".into() } else { "steam".into() },
            args: if cfg!(windows) {
                vec!["/C".into(), "start".into(), format!("steam://rungameid/{APP_ID}")]
            } else {
                vec![format!("steam://rungameid/{APP_ID}")]
            },
            env: vec![],
            description: "Launch via Steam (uses your saved launch options)".into(),
        }
    }

    /// Directly launch the discovered shipping exe with injection env applied.
    /// On Proton this is a placeholder: driving Proton directly needs
    /// STEAM_COMPAT_* wiring, which we finalize once the exe/prefix are known.
    pub fn direct(install: &GameInstall, exe: PathBuf) -> Self {
        let strategy = InjectionStrategy::for_runtime(&install.runtime);
        match &install.runtime {
            Runtime::WindowsNative => LaunchPlan {
                program: exe.display().to_string(),
                args: vec![],
                env: strategy.env,
                description: "Direct native launch with UE4SS proxy in place".into(),
            },
            Runtime::Proton { prefix } => LaunchPlan {
                program: exe.display().to_string(),
                args: vec![],
                // TODO(tomorrow): wrap with the correct Proton runtime + set
                // STEAM_COMPAT_DATA_PATH / STEAM_COMPAT_CLIENT_INSTALL_PATH.
                env: {
                    let mut env = strategy.env;
                    env.push((
                        "STEAM_COMPAT_DATA_PATH".into(),
                        prefix
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    ));
                    env
                },
                description: "Direct Proton launch (Proton wrapper wiring pending — \
                              prefer `via_steam` until confirmed)"
                    .into(),
            },
        }
    }

    /// Run it. With `dry_run`, only returns the description without spawning.
    pub fn run(&self, dry_run: bool) -> Result<Option<u32>, HostError> {
        if dry_run {
            return Ok(None);
        }
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let child = cmd.spawn()?;
        Ok(Some(child.id()))
    }
}

/// Best-effort discovery of the shipping executable under `Binaries/Win64`.
/// UE names it `<Project>-Win64-Shipping.exe` (or just `<Project>.exe`).
/// Confirmed against the real install tomorrow.
pub fn find_shipping_exe(install: &GameInstall) -> Option<PathBuf> {
    let win64 = install.layout.win64_dir();
    let entries = std::fs::read_dir(&win64).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false)
        })
        .filter(|p| {
            // Skip known non-game executables.
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            !name.contains("crashreport") && !name.contains("eac") && !name.contains("prereq")
        })
        .collect();
    // Prefer a *-Shipping.exe if present, else the first candidate.
    candidates.sort_by_key(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
        !name.contains("shipping") // false (shipping) sorts before true
    });
    candidates.into_iter().next()
}
