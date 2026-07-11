//! aml-core — pure, deterministic logic for the Aincrad mod loader.
//!
//! This crate has NO I/O. It owns the mod-manifest schema, the load-order
//! resolver, and the deploy-planner. Everything game- or filesystem-specific
//! lives in `aml-host`. Keeping this layer pure means the interesting logic is
//! unit-testable today, before the game is even installed.

pub mod error;
pub mod layout;
pub mod manifest;
pub mod order;
pub mod plan;

pub use error::CoreError;
pub use layout::InstallLayout;
pub use manifest::{ModId, ModKind, ModManifest, Profile};
pub use order::resolve_order;
pub use plan::{Conflict, ContentConflict, DeployManifest, DeployPlan, DeployedMod, FileOp};
