//! aml-host — the I/O boundary layer.
//!
//! Everything that touches the filesystem, the Steam install, the OS, or the
//! running game lives here. It is deliberately thin: the interesting decisions
//! are made in `aml-core`, and this crate just carries them out.

pub mod config;
pub mod deploy;
pub mod detect;
pub mod inject;
pub mod launch;
pub mod library;
pub mod nxm;
pub mod paths;
pub mod profiles;
pub mod ue4ss;

pub use paths::AppPaths;

pub use detect::{find_game, GameInstall, Runtime};
pub use inject::InjectionStrategy;
pub use library::{ModLibrary, ProfileState};
pub use profiles::Profiles;
pub use ue4ss::Ue4ssStatus;

/// Steam app id for Echoes of Aincrad (from the store URL /app/2244210/).
pub const APP_ID: u32 = 2244210;

/// Human-facing name of the game directory under `steamapps/common`.
pub const GAME_DIR_NAME: &str = "Echoes of Aincrad";

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Core(#[from] aml_core::CoreError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not locate the game install (looked in Steam library folders)")]
    GameNotFound,
    #[error("UE project folder not found under {0}; is the game fully installed?")]
    ProjectNotFound(String),
    #[error("{0}")]
    Other(String),
}
