//! Deterministic load-order resolution.
//!
//! Load order is a topological sort over `requires` edges, with ties broken
//! deterministically by (effective priority, then id). Determinism matters:
//! the same profile must always produce the same order, or mod conflicts become
//! irreproducible.

use crate::error::CoreError;
use crate::manifest::{ModId, Profile, ProfileEntry};
use std::collections::{BTreeMap, BTreeSet};

/// Resolve the enabled mods in a profile into a deterministic load order.
///
/// Earlier in the returned Vec == loaded earlier. Returns an error on missing
/// dependencies, dependency cycles, or duplicate ids.
pub fn resolve_order(profile: &Profile) -> Result<Vec<&ProfileEntry>, CoreError> {
    let enabled: Vec<&ProfileEntry> = profile.enabled().collect();

    // Index by id, rejecting duplicates.
    let mut by_id: BTreeMap<&ModId, &ProfileEntry> = BTreeMap::new();
    for e in &enabled {
        if by_id.insert(&e.manifest.id, e).is_some() {
            return Err(CoreError::DuplicateMod(e.manifest.id.clone()));
        }
    }

    // Validate dependencies point at enabled, present mods.
    for e in &enabled {
        for dep in &e.manifest.requires {
            if !by_id.contains_key(dep) {
                return Err(CoreError::MissingDependency(
                    e.manifest.id.clone(),
                    dep.clone(),
                ));
            }
        }
    }

    // Kahn's algorithm with a deterministic tiebreak. We pick the "ready" node
    // with the lowest (priority, id) each step so the output is stable.
    let mut indegree: BTreeMap<&ModId, usize> = by_id.keys().map(|id| (*id, 0usize)).collect();
    for e in &enabled {
        for _dep in &e.manifest.requires {
            *indegree.get_mut(&e.manifest.id).unwrap() += 1;
        }
    }

    // Sort key for tiebreaking ready nodes.
    let sort_key = |id: &ModId| -> (i32, ModId) {
        let e = by_id[id];
        (e.effective_priority(), id.clone())
    };

    let mut ready: BTreeSet<(i32, ModId)> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(id, _)| sort_key(id))
        .collect();

    let mut order: Vec<&ProfileEntry> = Vec::with_capacity(enabled.len());
    while let Some(key) = ready.iter().next().cloned() {
        ready.remove(&key);
        let id = key.1;
        order.push(by_id[&id]);

        // Decrement dependents. A dependent is anyone that `requires` this id.
        for e in &enabled {
            if e.manifest.requires.contains(&id) {
                let d = indegree.get_mut(&e.manifest.id).unwrap();
                *d -= 1;
                if *d == 0 {
                    ready.insert(sort_key(&e.manifest.id));
                }
            }
        }
    }

    if order.len() != enabled.len() {
        // Whatever is left has a nonzero indegree => part of a cycle.
        let cycle: Vec<String> = indegree
            .iter()
            .filter(|(_, &d)| d > 0)
            .map(|(id, _)| id.0.clone())
            .collect();
        return Err(CoreError::DependencyCycle(cycle));
    }

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ModKind, ModManifest};

    fn entry(id: &str, priority: i32, requires: &[&str]) -> ProfileEntry {
        ProfileEntry {
            manifest: ModManifest {
                id: id.into(),
                name: id.to_string(),
                version: "1.0.0".into(),
                kind: ModKind::Pak,
                priority,
                requires: requires.iter().map(|s| (*s).into()).collect(),
                author: None,
                description: None,
            },
            enabled: true,
            priority_override: None,
        }
    }

    fn ids(order: &[&ProfileEntry]) -> Vec<String> {
        order.iter().map(|e| e.manifest.id.0.clone()).collect()
    }

    #[test]
    fn priority_orders_independent_mods() {
        let profile = Profile {
            name: "t".into(),
            mods: vec![entry("b", 10, &[]), entry("a", 5, &[]), entry("c", 5, &[])],
        };
        let order = resolve_order(&profile).unwrap();
        // priority 5 before 10; within 5, id "a" before "c".
        assert_eq!(ids(&order), vec!["a", "c", "b"]);
    }

    #[test]
    fn dependency_forces_predecessor_first() {
        // "hi" has lower priority number (would sort first) but requires "lo".
        let profile = Profile {
            name: "t".into(),
            mods: vec![entry("hi", 0, &["lo"]), entry("lo", 100, &[])],
        };
        let order = resolve_order(&profile).unwrap();
        assert_eq!(ids(&order), vec!["lo", "hi"]);
    }

    #[test]
    fn missing_dependency_errors() {
        let profile = Profile {
            name: "t".into(),
            mods: vec![entry("a", 0, &["ghost"])],
        };
        assert!(matches!(
            resolve_order(&profile),
            Err(CoreError::MissingDependency(_, _))
        ));
    }

    #[test]
    fn cycle_errors() {
        let profile = Profile {
            name: "t".into(),
            mods: vec![entry("a", 0, &["b"]), entry("b", 0, &["a"])],
        };
        assert!(matches!(
            resolve_order(&profile),
            Err(CoreError::DependencyCycle(_))
        ));
    }

    #[test]
    fn disabled_mods_excluded() {
        let mut e = entry("off", 0, &[]);
        e.enabled = false;
        let profile = Profile {
            name: "t".into(),
            mods: vec![e, entry("on", 0, &[])],
        };
        let order = resolve_order(&profile).unwrap();
        assert_eq!(ids(&order), vec!["on"]);
    }

    #[test]
    fn deterministic_across_runs() {
        let profile = Profile {
            name: "t".into(),
            mods: vec![
                entry("d", 1, &["a"]),
                entry("a", 1, &[]),
                entry("c", 1, &["a"]),
                entry("b", 1, &["a"]),
            ],
        };
        let first = ids(&resolve_order(&profile).unwrap());
        for _ in 0..50 {
            assert_eq!(ids(&resolve_order(&profile).unwrap()), first);
        }
        assert_eq!(first, vec!["a", "b", "c", "d"]);
    }
}
