//! Error types for the core layer. Stable, human-readable reasons.

use crate::manifest::ModId;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("mod '{0}' depends on '{1}', which is not present or not enabled")]
    MissingDependency(ModId, ModId),

    #[error("dependency cycle detected involving: {}", .0.join(" -> "))]
    DependencyCycle(Vec<String>),

    #[error("duplicate mod id '{0}' in profile")]
    DuplicateMod(ModId),

    #[error("mod '{0}' has kind {1} but the install layout does not support it yet")]
    UnsupportedKind(ModId, &'static str),
}
