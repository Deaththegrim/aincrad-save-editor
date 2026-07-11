//! Deploy planning: turn a resolved profile + install layout into an ordered
//! list of filesystem operations. PURE — computes *what* to do; `aml-host`
//! executes it. This split makes the interesting logic (naming, ordering,
//! UE4SS registration) testable with zero I/O, and gives us a free `--dry-run`.

use crate::error::CoreError;
use crate::layout::InstallLayout;
use crate::manifest::{ModId, ModKind, ProfileEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single filesystem action the host must perform to deploy a mod set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    /// Ensure a directory exists.
    EnsureDir { path: PathBuf },
    /// Copy a mod's pak payload from its staging dir into `~mods`, renamed with
    /// a zero-padded order prefix so UE mounts mods in resolved order.
    DeployPak {
        mod_id: ModId,
        /// Filename to give the pak in `~mods`, e.g. `000_author.mod_P.pak`.
        dest_name: String,
        dest: PathBuf,
    },
    /// Copy a UE4SS Lua/C++ mod's folder into `ue4ss/Mods/<name>`.
    DeployUe4ssMod {
        mod_id: ModId,
        dest_dir: PathBuf,
    },
    /// Ensure these mod names are present+enabled in the UE4SS `mods.txt`,
    /// MERGING with any existing entries (built-in mods, keybinds) rather than
    /// overwriting them. The host reads the current file and upserts.
    RegisterUe4ssMods {
        path: PathBuf,
        names: Vec<String>,
    },
    /// Remove a file or directory that aml previously deployed. Only ever
    /// targets paths recorded in aml's own deploy manifest, so it never touches
    /// built-in or user-managed files.
    RemovePath { path: PathBuf },
    /// Remove these mod names from the UE4SS `mods.txt` (and sibling mods.json),
    /// leaving all other entries intact.
    UnregisterUe4ssMods {
        path: PathBuf,
        names: Vec<String>,
    },
}

/// Record of one mod aml placed into an install, used to reconcile later
/// deploys (remove what's no longer wanted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployedMod {
    pub id: ModId,
    pub kind: ModKind,
    /// The files/dirs aml created for this mod (paks, UE4SS mod folders).
    pub paths: Vec<PathBuf>,
    /// The UE4SS registry name, for lua/cpp mods.
    #[serde(default)]
    pub ue4ss_name: Option<String>,
}

/// What aml has deployed into an install. Persisted at
/// [`InstallLayout::deploy_manifest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployManifest {
    pub mods: Vec<DeployedMod>,
}

/// An ordered plan plus the manifest of what it will have deployed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeployPlan {
    pub ops: Vec<FileOp>,
    /// What the install will contain (from aml) after this plan runs. The host
    /// persists this so the next deploy can reconcile.
    pub manifest: DeployManifest,
}

impl DeployPlan {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Build a deploy plan from the already-resolved load order.
///
/// `order` must come from [`crate::order::resolve_order`] so indices reflect the
/// real load order. `pak_source_name` maps a mod id to the pak filename staged
/// on disk (the host knows this; core stays pure).
pub fn plan_deploy(
    order: &[&ProfileEntry],
    layout: &InstallLayout,
) -> Result<DeployPlan, CoreError> {
    let mut ops = Vec::new();

    ops.push(FileOp::EnsureDir {
        path: layout.mods_dir(),
    });
    ops.push(FileOp::EnsureDir {
        path: layout.logic_mods_dir(),
    });
    ops.push(FileOp::EnsureDir {
        path: layout.ue4ss_mods_dir(),
    });

    let mut ue4ss_names: Vec<String> = Vec::new();
    let mut manifest = DeployManifest::default();

    for (idx, entry) in order.iter().enumerate() {
        let id = &entry.manifest.id;
        match entry.manifest.kind {
            ModKind::Pak => {
                // UE mounts paks alphabetically; a zero-padded prefix pins the
                // resolved order. `_P` suffix marks it as a patch pak.
                let dest_name = format!("{idx:03}_{}_P.pak", sanitize(&id.0));
                let dest = layout.mods_dir().join(&dest_name);
                ops.push(FileOp::DeployPak {
                    mod_id: id.clone(),
                    dest_name,
                    dest: dest.clone(),
                });
                manifest.mods.push(DeployedMod {
                    id: id.clone(),
                    kind: ModKind::Pak,
                    paths: vec![dest],
                    ue4ss_name: None,
                });
            }
            ModKind::Logic => {
                // Blueprint LogicMods live in their own folder; the UE4SS BP
                // loader mounts all of them (order-prefix kept for determinism).
                let dest_name = format!("{idx:03}_{}_P.pak", sanitize(&id.0));
                let dest = layout.logic_mods_dir().join(&dest_name);
                ops.push(FileOp::DeployPak {
                    mod_id: id.clone(),
                    dest_name,
                    dest: dest.clone(),
                });
                manifest.mods.push(DeployedMod {
                    id: id.clone(),
                    kind: ModKind::Logic,
                    paths: vec![dest],
                    ue4ss_name: None,
                });
            }
            ModKind::Lua | ModKind::Cpp => {
                let folder = ue4ss_name(&id.0);
                let dest_dir = layout.ue4ss_mods_dir().join(&folder);
                ops.push(FileOp::DeployUe4ssMod {
                    mod_id: id.clone(),
                    dest_dir: dest_dir.clone(),
                });
                ue4ss_names.push(folder.clone());
                manifest.mods.push(DeployedMod {
                    id: id.clone(),
                    kind: entry.manifest.kind,
                    paths: vec![dest_dir],
                    ue4ss_name: Some(folder),
                });
            }
        }
    }

    if !ue4ss_names.is_empty() {
        ops.push(FileOp::RegisterUe4ssMods {
            path: layout.ue4ss_mods_txt(),
            names: ue4ss_names,
        });
    }

    Ok(DeployPlan { ops, manifest })
}

/// A detected clash between two mods in a planned deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub a: ModId,
    pub b: ModId,
    pub reason: String,
}

/// Two mods whose pak payloads both contain the same internal asset path(s).
/// Unlike a [`Conflict`] (same *deploy* destination), both paks mount fine — but
/// UE resolves a duplicated asset path to whichever pak mounts last, so the
/// lower-priority mod's version is silently shadowed. The load order decides the
/// winner; this just surfaces that a decision is being made at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentConflict {
    /// Earlier in load order (loses: its asset is shadowed).
    pub a: ModId,
    /// Later in load order (wins: its asset is the one the game sees).
    pub b: ModId,
    /// Internal asset paths both paks provide.
    pub shared_paths: Vec<String>,
}

/// Detect paks that override the same internal asset path. `contents` maps each
/// mod id to the logical file paths inside its pak, IN LOAD ORDER (index 0 mounts
/// first / loses ties). Pure: the host lists pak contents and hands them here.
///
/// For each overlapping pair, `b` is whichever mod appears later in `order` — the
/// one whose asset actually wins — so the report reads "a is shadowed by b".
pub fn detect_content_conflicts(
    order: &[ModId],
    contents: &std::collections::BTreeMap<ModId, Vec<String>>,
) -> Vec<ContentConflict> {
    use std::collections::BTreeMap;

    // Load-order rank per mod; missing mods sort last (shouldn't happen).
    let rank = |id: &ModId| order.iter().position(|o| o == id).unwrap_or(usize::MAX);

    // path -> every mod providing it.
    let mut providers: BTreeMap<&str, Vec<&ModId>> = BTreeMap::new();
    for id in order {
        if let Some(paths) = contents.get(id) {
            for p in paths {
                providers.entry(p.as_str()).or_default().push(id);
            }
        }
    }

    // Accumulate shared paths per ordered (loser, winner) pair.
    let mut pairs: BTreeMap<(ModId, ModId), Vec<String>> = BTreeMap::new();
    for (path, mods) in providers {
        if mods.len() < 2 {
            continue;
        }
        for (i, &m1) in mods.iter().enumerate() {
            for &m2 in &mods[i + 1..] {
                let (loser, winner) = if rank(m1) <= rank(m2) {
                    (m1.clone(), m2.clone())
                } else {
                    (m2.clone(), m1.clone())
                };
                pairs.entry((loser, winner)).or_default().push(path.to_string());
            }
        }
    }

    pairs
        .into_iter()
        .map(|((a, b), mut shared_paths)| {
            shared_paths.sort();
            ContentConflict { a, b, shared_paths }
        })
        .collect()
}

/// Detect mods that would deploy to the same destination — e.g. two ids that
/// map to the same UE4SS folder (`a.b` and `a-b` both sanitize to `a_b`), so one
/// would clobber the other. Pure: works off a computed manifest.
pub fn detect_conflicts(manifest: &DeployManifest) -> Vec<Conflict> {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<PathBuf, ModId> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for m in &manifest.mods {
        for path in &m.paths {
            if let Some(prev) = seen.get(path) {
                conflicts.push(Conflict {
                    a: prev.clone(),
                    b: m.id.clone(),
                    reason: format!("both deploy to {}", path.display()),
                });
            } else {
                seen.insert(path.clone(), m.id.clone());
            }
        }
    }
    conflicts
}

/// Compute the cleanup needed to go from a previous deployment to the next one:
/// remove files/registry entries for mods that are gone, and stale paths for
/// mods whose deploy target changed (e.g. a pak renamed by a new load order).
///
/// Pure: only references paths recorded in `prev`, so it can never target
/// built-in or user files.
pub fn plan_reconcile(
    prev: &DeployManifest,
    next: &DeployManifest,
    layout: &InstallLayout,
) -> Vec<FileOp> {
    let mut ops = Vec::new();
    let mut unregister: Vec<String> = Vec::new();

    for pm in &prev.mods {
        match next.mods.iter().find(|m| m.id == pm.id) {
            None => {
                // Mod fully gone: remove every path + its registry entry.
                for p in &pm.paths {
                    ops.push(FileOp::RemovePath { path: p.clone() });
                }
                if let Some(name) = &pm.ue4ss_name {
                    unregister.push(name.clone());
                }
            }
            Some(nm) => {
                // Still deployed: drop any old path the new deploy won't recreate.
                for p in &pm.paths {
                    if !nm.paths.contains(p) {
                        ops.push(FileOp::RemovePath { path: p.clone() });
                    }
                }
                // Registry name changed (e.g. id rename): unregister the old one.
                if let Some(old) = &pm.ue4ss_name {
                    if nm.ue4ss_name.as_ref() != Some(old) {
                        unregister.push(old.clone());
                    }
                }
            }
        }
    }

    if !unregister.is_empty() {
        ops.push(FileOp::UnregisterUe4ssMods {
            path: layout.ue4ss_mods_txt(),
            names: unregister,
        });
    }

    ops
}

/// Make a mod id safe for use as a filename/folder component.
fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            c
        } else {
            '_'
        })
        .collect()
}

/// UE4SS mod folder name: the mods.txt/mods.json entry must exactly match the
/// folder, and every known-working UE4SS mod uses [A-Za-z0-9_] names. Map our
/// dotted library ids (author.mod) to underscore style for the deployed folder.
fn ue4ss_name(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ModManifest;

    fn entry(id: &str, kind: ModKind) -> ProfileEntry {
        ProfileEntry {
            manifest: ModManifest {
                id: id.into(),
                name: id.to_string(),
                version: "1.0.0".into(),
                kind,
                priority: 0,
                requires: vec![],
                author: None,
                description: None,
            },
            enabled: true,
            priority_override: None,
        }
    }

    fn layout() -> InstallLayout {
        InstallLayout::new("/games/EoA", "EchoesOfAincrad")
    }

    #[test]
    fn pak_mods_get_ordered_prefixes() {
        let a = entry("author.a", ModKind::Pak);
        let b = entry("author.b", ModKind::Pak);
        let order = vec![&a, &b];
        let plan = plan_deploy(&order, &layout()).unwrap();

        let names: Vec<String> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                FileOp::DeployPak { dest_name, .. } => Some(dest_name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["000_author.a_P.pak", "001_author.b_P.pak"]);
    }

    #[test]
    fn lua_mod_registers_in_mods_txt() {
        let l = entry("author.script", ModKind::Lua);
        let order = vec![&l];
        let plan = plan_deploy(&order, &layout()).unwrap();

        let names = plan.ops.iter().find_map(|op| match op {
            FileOp::RegisterUe4ssMods { names, .. } => Some(names.clone()),
            _ => None,
        });
        // Dotted id -> underscore UE4SS folder/name (dots break UE4SS mod loading).
        assert_eq!(names, Some(vec!["author_script".to_string()]));
    }

    #[test]
    fn lua_mod_folder_is_dot_free() {
        let l = entry("author.script", ModKind::Lua);
        let order = vec![&l];
        let plan = plan_deploy(&order, &layout()).unwrap();
        let dest = plan.ops.iter().find_map(|op| match op {
            FileOp::DeployUe4ssMod { dest_dir, .. } => Some(dest_dir.clone()),
            _ => None,
        });
        assert!(dest.unwrap().ends_with("author_script"));
    }

    #[test]
    fn deploy_targets_land_under_install() {
        let p = entry("x", ModKind::Pak);
        let order = vec![&p];
        let plan = plan_deploy(&order, &layout()).unwrap();
        let dest = plan.ops.iter().find_map(|op| match op {
            FileOp::DeployPak { dest, .. } => Some(dest.clone()),
            _ => None,
        });
        assert_eq!(
            dest,
            Some(PathBuf::from(
                "/games/EoA/EchoesOfAincrad/Content/Paks/~mods/000_x_P.pak"
            ))
        );
    }

    #[test]
    fn logic_mods_land_in_logicmods_dir() {
        let l = entry("author.bp", ModKind::Logic);
        let order = vec![&l];
        let plan = plan_deploy(&order, &layout()).unwrap();
        let dest = plan.ops.iter().find_map(|op| match op {
            FileOp::DeployPak { dest, .. } => Some(dest.clone()),
            _ => None,
        });
        assert_eq!(
            dest,
            Some(PathBuf::from(
                "/games/EoA/EchoesOfAincrad/Content/Paks/LogicMods/000_author.bp_P.pak"
            ))
        );
    }

    #[test]
    fn plan_deploy_records_manifest() {
        let p = entry("author.p", ModKind::Pak);
        let l = entry("author.l", ModKind::Lua);
        let order = vec![&p, &l];
        let plan = plan_deploy(&order, &layout()).unwrap();
        assert_eq!(plan.manifest.mods.len(), 2);
        let lua = plan.manifest.mods.iter().find(|m| m.kind == ModKind::Lua).unwrap();
        assert_eq!(lua.ue4ss_name.as_deref(), Some("author_l"));
        assert!(lua.paths[0].ends_with("author_l"));
    }

    #[test]
    fn reconcile_removes_dropped_mod() {
        let l = layout();
        // Previously deployed a pak + a lua mod.
        let p = entry("author.p", ModKind::Pak);
        let lua = entry("author.l", ModKind::Lua);
        let prev = plan_deploy(&[&p, &lua], &l).unwrap().manifest;
        // Next deploy has only the pak.
        let next = plan_deploy(&[&p], &l).unwrap().manifest;

        let ops = plan_reconcile(&prev, &next, &l);
        // The lua mod's dir is removed and it's unregistered.
        assert!(ops.iter().any(|op| matches!(op,
            FileOp::RemovePath { path } if path.ends_with("author_l"))));
        assert!(ops.iter().any(|op| matches!(op,
            FileOp::UnregisterUe4ssMods { names, .. } if names == &vec!["author_l".to_string()])));
    }

    #[test]
    fn reconcile_removes_stale_pak_on_reorder() {
        let l = layout();
        let a = entry("a", ModKind::Pak);
        let b = entry("b", ModKind::Pak);
        // prev: [a, b] -> a is 000, b is 001
        let prev = plan_deploy(&[&a, &b], &l).unwrap().manifest;
        // next: [b, a] -> b is 000, a is 001; a's old 000 pak is now stale.
        let next = plan_deploy(&[&b, &a], &l).unwrap().manifest;
        let ops = plan_reconcile(&prev, &next, &l);
        assert!(ops.iter().any(|op| matches!(op,
            FileOp::RemovePath { path } if path.to_string_lossy().contains("000_a_P.pak"))));
    }

    #[test]
    fn detects_ue4ss_name_collision() {
        // "a.mod" and "a-mod" both sanitize to the UE4SS folder "a_mod".
        let x = entry("a.mod", ModKind::Lua);
        let y = entry("a-mod", ModKind::Lua);
        let manifest = plan_deploy(&[&x, &y], &layout()).unwrap().manifest;
        let conflicts = detect_conflicts(&manifest);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].reason.contains("a_mod"));
    }

    #[test]
    fn no_conflict_for_distinct_mods() {
        let x = entry("a", ModKind::Lua);
        let y = entry("b", ModKind::Pak);
        let manifest = plan_deploy(&[&x, &y], &layout()).unwrap().manifest;
        assert!(detect_conflicts(&manifest).is_empty());
    }

    #[test]
    fn reconcile_noop_when_unchanged() {
        let l = layout();
        let a = entry("a", ModKind::Pak);
        let m = plan_deploy(&[&a], &l).unwrap().manifest;
        assert!(plan_reconcile(&m, &m, &l).is_empty());
    }

    #[test]
    fn content_conflict_reports_shared_asset_winner() {
        use std::collections::BTreeMap;
        let a = ModId::from("author.a");
        let b = ModId::from("author.b");
        // Load order: a first, b second -> b wins the shared asset.
        let order = vec![a.clone(), b.clone()];
        let mut contents = BTreeMap::new();
        contents.insert(a.clone(), vec![
            "Game/UI/Widget.uasset".to_string(),
            "Game/Only/A.uasset".to_string(),
        ]);
        contents.insert(b.clone(), vec![
            "Game/UI/Widget.uasset".to_string(),
            "Game/Only/B.uasset".to_string(),
        ]);
        let conflicts = detect_content_conflicts(&order, &contents);
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.a, a); // earlier -> shadowed
        assert_eq!(c.b, b); // later -> wins
        assert_eq!(c.shared_paths, vec!["Game/UI/Widget.uasset".to_string()]);
    }

    #[test]
    fn content_conflict_winner_follows_load_order() {
        use std::collections::BTreeMap;
        let a = ModId::from("author.a");
        let b = ModId::from("author.b");
        // Reverse the order: b first, a second -> a now wins.
        let order = vec![b.clone(), a.clone()];
        let mut contents = BTreeMap::new();
        contents.insert(a.clone(), vec!["Shared.uasset".to_string()]);
        contents.insert(b.clone(), vec!["Shared.uasset".to_string()]);
        let conflicts = detect_content_conflicts(&order, &contents);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].a, b); // b loads first -> shadowed
        assert_eq!(conflicts[0].b, a); // a loads last -> wins
    }

    #[test]
    fn no_content_conflict_for_disjoint_paks() {
        use std::collections::BTreeMap;
        let a = ModId::from("a");
        let b = ModId::from("b");
        let order = vec![a.clone(), b.clone()];
        let mut contents = BTreeMap::new();
        contents.insert(a.clone(), vec!["X.uasset".to_string()]);
        contents.insert(b.clone(), vec!["Y.uasset".to_string()]);
        assert!(detect_content_conflicts(&order, &contents).is_empty());
    }

    #[test]
    fn content_conflict_three_way_produces_all_pairs() {
        use std::collections::BTreeMap;
        let a = ModId::from("a");
        let b = ModId::from("b");
        let c = ModId::from("c");
        let order = vec![a.clone(), b.clone(), c.clone()];
        let mut contents = BTreeMap::new();
        for id in [&a, &b, &c] {
            contents.insert((*id).clone(), vec!["Shared.uasset".to_string()]);
        }
        // a⇄b, a⇄c, b⇄c
        assert_eq!(detect_content_conflicts(&order, &contents).len(), 3);
    }

    #[test]
    fn empty_profile_still_ensures_dirs() {
        let order: Vec<&ProfileEntry> = vec![];
        let plan = plan_deploy(&order, &layout()).unwrap();
        assert!(plan
            .ops
            .iter()
            .any(|op| matches!(op, FileOp::EnsureDir { .. })));
    }
}
