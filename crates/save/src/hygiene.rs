//! Mod-item save hygiene: find (and optionally purge) item references whose
//! (category, id) is not in the base-game catalogs — i.e. modded/stale ids
//! left behind by an uninstalled mod, which the game may not tolerate.
//!
//! NOT wired into the editor UI (appearance-only scope). Consumed by the
//! `scan_items` example and by tests. Catalogs come from the
//! echoes-of-aincrad-mods `docs/data` TSVs (weapons / armor-shield / items).
//!
//! Invariants:
//! - Purge is REMOVAL-ONLY and refuses to run if any stale item is currently
//!   equipped (Equipment.*UniqueID would dangle) — unequip in-game first.
//! - Purge of a clean save is a no-op; inject-then-purge round-trips
//!   byte-identical (pinned by test).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use uesave::{Properties, Property, Save, StructValue, ValueVec};

/// Per-category sets of valid base-game item ids.
pub struct Catalog(HashMap<String, HashSet<i64>>);

impl Catalog {
    /// Load from a `docs/data` dir holding weapons.tsv / armor-shield.tsv /
    /// items.tsv (first two columns = Category, ID).
    pub fn from_dir(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut all: HashMap<String, HashSet<i64>> = HashMap::new();
        for f in ["weapons.tsv", "armor-shield.tsv", "items.tsv"] {
            let text = std::fs::read_to_string(dir.as_ref().join(f))?;
            for line in text.lines().skip(1) {
                let mut cols = line.split('\t');
                let (Some(cat), Some(id)) = (cols.next(), cols.next()) else { continue };
                if let Ok(id) = id.parse::<i64>() {
                    all.entry(cat.to_string()).or_default().insert(id);
                }
            }
        }
        Ok(Catalog(all))
    }

    /// EItemCategory suffix (save enum) -> catalog category name.
    /// None = a category the catalogs don't cover (reported, never purged).
    fn map_enum(suffix: &str) -> Option<&'static str> {
        Some(match suffix {
            "Cost" => "Use",
            "Material" => "Material",
            "Heal" => "Heal",
            "Col" => "Col",
            "Sphere" => "Sphere",
            "KeyItem" => "Key",
            "OneHandedSword" => "OneHandedSword",
            "Rapier" => "Rapier",
            "Dagger" => "Dagger",
            "Mace" => "Mace",
            "TwoHandedSword" => "TwoHandedSword",
            "Axe" => "Axe",
            "Shield" => "Shield",
            "Upper" => "Upper",
            "Gloves" => "Glove",
            "Lower" => "Lower",
            _ => return None,
        })
    }

    /// RecipeLists field name -> catalog category for its item ids
    /// (recipe-list entries ARE item ids — proven by the recipe-inject work).
    fn map_recipe_list(field: &str) -> Option<&'static str> {
        Some(match field {
            "UsableRecipeList" => "Use",
            "OneHandedSwordRecipeList" => "OneHandedSword",
            "RapierRecipeList" => "Rapier",
            "DaggerRecipeList" => "Dagger",
            "MaceRecipeList" => "Mace",
            "TwoHandedSwordRecipeList" => "TwoHandedSword",
            "AxeRecipeList" => "Axe",
            "UpperRecipeList" => "Upper",
            "GlovesRecipeList" => "Glove",
            "LowerRecipeList" => "Lower",
            "ShieldRecipeList" => "Shield",
            _ => return None, // Claw/Scimitar: cut content, no catalog
        })
    }

    fn known(&self, cat: &str, id: i64) -> bool {
        self.0.get(cat).is_some_and(|s| s.contains(&id))
    }
}

/// One stale (unknown-id) reference found in the save.
#[derive(Debug, Clone)]
pub struct Stale {
    /// Property path, e.g. `.CharacterSaveData[0].WeaponChest[390]`.
    pub path: String,
    /// Catalog category the id was checked against.
    pub category: &'static str,
    /// The unknown item id.
    pub id: i64,
    /// Chest entry's UniqueID (None for recipe-list entries).
    pub unique_id: Option<i64>,
}

impl std::fmt::Display for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}  {} id {}", self.path, self.category, self.id)?;
        if let Some(uid) = self.unique_id {
            write!(f, "  (UniqueID {uid})")?;
        }
        Ok(())
    }
}

/// Scan result.
#[derive(Debug, Default)]
pub struct Report {
    pub stale: Vec<Stale>,
    /// Chest entries whose category enum has no catalog mapping.
    pub uncatalogued: Vec<String>,
    /// Every non-zero Equipment.*UniqueID reference (path, uid).
    pub equip_refs: Vec<(String, i64)>,
    /// Every UniqueID present in any chest.
    pub chest_uids: HashSet<i64>,
    /// Total chest entries visited.
    pub entries: usize,
}

impl Report {
    /// Equipment refs that resolve to no chest entry at all (already broken).
    pub fn dangling_equips(&self) -> Vec<&(String, i64)> {
        self.equip_refs.iter().filter(|(_, uid)| !self.chest_uids.contains(uid)).collect()
    }

    /// Equipment refs pointing at a STALE chest entry (purge would break them).
    pub fn equipped_stale(&self) -> Vec<&(String, i64)> {
        let stale_uids: HashSet<i64> =
            self.stale.iter().filter_map(|s| s.unique_id).collect();
        self.equip_refs.iter().filter(|(_, uid)| stale_uids.contains(uid)).collect()
    }
}

/// Purge refused: stale items are currently equipped (path, uid).
#[derive(Debug)]
pub struct PurgeBlocked(pub Vec<(String, i64)>);

impl std::fmt::Display for PurgeBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "stale item(s) currently equipped — unequip in-game first:")?;
        for (path, uid) in &self.0 {
            writeln!(f, "  {path} -> UniqueID {uid}")?;
        }
        Ok(())
    }
}

fn is_chest(name: &str) -> bool {
    name.ends_with("Chest") || name == "UserItems"
}

fn get_int(props: &Properties, name: &str) -> Option<i64> {
    props.0.iter().find(|(k, _)| k.1 == name).and_then(|(_, p)| match p {
        Property::Int(v) => Some(*v as i64),
        Property::Int64(v) => Some(*v),
        _ => None,
    })
}

fn get_enum<'p>(props: &'p Properties, name: &str) -> Option<&'p str> {
    props.0.iter().find(|(k, _)| k.1 == name).and_then(|(_, p)| match p {
        Property::Enum(s) => Some(s.as_str()),
        _ => None,
    })
}

/// (catalog category or None, id, raw enum) of a chest entry.
fn entry_cat_id(props: &Properties) -> Option<(Option<&'static str>, i64, &str)> {
    let id = get_int(props, "ItemId")?;
    let en = get_enum(props, "Category")?;
    let suffix = en.rsplit("ItemCategory_").next().unwrap_or(en);
    Some((Catalog::map_enum(suffix), id, en))
}

/// Read-only scan of the whole save tree.
pub fn scan(save: &Save, cat: &Catalog) -> Report {
    let mut r = Report::default();
    walk(&save.root.properties, cat, &mut r, "");
    r
}

fn walk(props: &Properties, cat: &Catalog, r: &mut Report, path: &str) {
    for (k, p) in props.0.iter() {
        let name = &k.1;
        match p {
            Property::Struct(StructValue::Struct(inner)) => {
                if name == "RecipeLists" {
                    scan_recipes(inner, cat, r, path);
                } else if name == "Equipment" {
                    scan_equipment(inner, r, path);
                }
                walk(inner, cat, r, &format!("{path}.{name}"));
            }
            Property::Array(ValueVec::Struct(v)) => {
                if is_chest(name) {
                    scan_chest(name, v, cat, r, path);
                }
                for (i, sv) in v.iter().enumerate() {
                    if let StructValue::Struct(inner) = sv {
                        walk(inner, cat, r, &format!("{path}.{name}[{i}]"));
                    }
                }
            }
            _ => {}
        }
    }
}

fn scan_chest(name: &str, v: &[StructValue], cat: &Catalog, r: &mut Report, path: &str) {
    for (i, sv) in v.iter().enumerate() {
        let StructValue::Struct(props) = sv else { continue };
        let Some((mapped, id, en)) = entry_cat_id(props) else { continue };
        r.entries += 1;
        let uid = get_int(props, "UniqueID");
        if let Some(uid) = uid {
            r.chest_uids.insert(uid);
        }
        match mapped {
            Some(ccat) => {
                if !cat.known(ccat, id) {
                    r.stale.push(Stale {
                        path: format!("{path}.{name}[{i}]"),
                        category: ccat,
                        id,
                        unique_id: uid,
                    });
                }
            }
            None => r.uncatalogued.push(format!("{path}.{name}[{i}]  enum {en} id {id}")),
        }
    }
}

fn scan_recipes(props: &Properties, cat: &Catalog, r: &mut Report, path: &str) {
    for (k, p) in props.0.iter() {
        let field = &k.1;
        let Property::Array(ValueVec::UInt16(ids)) = p else { continue };
        let Some(ccat) = Catalog::map_recipe_list(field) else { continue };
        for id in ids {
            if !cat.known(ccat, *id as i64) {
                r.stale.push(Stale {
                    path: format!("{path}.RecipeLists.{field}"),
                    category: ccat,
                    id: *id as i64,
                    unique_id: None,
                });
            }
        }
    }
}

fn scan_equipment(props: &Properties, r: &mut Report, path: &str) {
    for (k, p) in props.0.iter() {
        if !k.1.ends_with("UniqueID") {
            continue;
        }
        if let Property::Int(v) = p {
            if *v != 0 {
                r.equip_refs.push((format!("{path}.Equipment.{}", k.1), *v as i64));
            }
        }
    }
}

/// Remove every stale chest entry and stale recipe id. Refuses (no mutation)
/// if any stale item is currently equipped. Returns descriptions of removals.
pub fn purge(save: &mut Save, cat: &Catalog) -> Result<Vec<String>, PurgeBlocked> {
    let before = scan(save, cat);
    let equipped = before.equipped_stale();
    if !equipped.is_empty() {
        return Err(PurgeBlocked(equipped.into_iter().cloned().collect()));
    }
    let mut removed = Vec::new();
    purge_walk(&mut save.root.properties, cat, &mut removed, "");
    Ok(removed)
}

fn purge_walk(props: &mut Properties, cat: &Catalog, removed: &mut Vec<String>, path: &str) {
    for (k, p) in props.0.iter_mut() {
        let name = k.1.clone();
        match p {
            Property::Struct(StructValue::Struct(inner)) => {
                if name == "RecipeLists" {
                    for (rk, rp) in inner.0.iter_mut() {
                        let field = rk.1.clone();
                        let Property::Array(ValueVec::UInt16(ids)) = rp else { continue };
                        let Some(ccat) = Catalog::map_recipe_list(&field) else { continue };
                        ids.retain(|id| {
                            let keep = cat.known(ccat, *id as i64);
                            if !keep {
                                removed.push(format!(
                                    "{path}.RecipeLists.{field}  {ccat} recipe id {id}"
                                ));
                            }
                            keep
                        });
                    }
                }
                purge_walk(inner, cat, removed, &format!("{path}.{name}"));
            }
            Property::Array(ValueVec::Struct(v)) => {
                if is_chest(&name) {
                    v.retain(|sv| {
                        let StructValue::Struct(entry) = sv else { return true };
                        let Some((Some(ccat), id, _)) = entry_cat_id(entry) else { return true };
                        let keep = cat.known(ccat, id);
                        if !keep {
                            removed.push(format!("{path}.{name}  {ccat} id {id}"));
                        }
                        keep
                    });
                }
                for sv in v.iter_mut() {
                    if let StructValue::Struct(inner) = sv {
                        purge_walk(inner, cat, removed, &format!("{path}.{name}[]"));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Self-test helper: plant a synthetic stale weapon (id 999, UniqueID 999999,
/// cloned from WeaponChest[0]) + a stale dagger recipe id (38) — the exact
/// "mod uninstalled, items left behind" state. Returns what was injected.
#[doc(hidden)]
pub fn inject_test_items(save: &mut Save) -> Vec<String> {
    let mut injected = Vec::new();
    inject_walk(&mut save.root.properties, &mut injected);
    injected
}

fn inject_walk(props: &mut Properties, injected: &mut Vec<String>) {
    for (k, p) in props.0.iter_mut() {
        let name = k.1.clone();
        match p {
            Property::Struct(StructValue::Struct(inner)) => {
                if name == "RecipeLists" {
                    for (rk, rp) in inner.0.iter_mut() {
                        if rk.1 == "DaggerRecipeList" {
                            if let Property::Array(ValueVec::UInt16(ids)) = rp {
                                ids.push(38);
                                injected.push("DaggerRecipeList += recipe id 38".into());
                            }
                        }
                    }
                }
                inject_walk(inner, injected);
            }
            Property::Array(ValueVec::Struct(v)) => {
                if name == "WeaponChest" && !v.is_empty() {
                    if let StructValue::Struct(first) = &v[0] {
                        let mut fake = first.clone();
                        for (fk, fp) in fake.0.iter_mut() {
                            match (fk.1.as_str(), &mut *fp) {
                                ("ItemId", Property::Int(x)) => *x = 999,
                                ("UniqueID", Property::Int(x)) => *x = 999_999,
                                _ => {}
                            }
                        }
                        v.push(StructValue::Struct(fake));
                        injected
                            .push("WeaponChest += fake item id 999 (UniqueID 999999)".into());
                    }
                } else {
                    for sv in v.iter_mut() {
                        if let StructValue::Struct(inner) = sv {
                            inject_walk(inner, injected);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SaveFile;
    use std::path::PathBuf;

    /// Gate: local save + key + catalog TSVs, or skip (CI / other machines).
    fn local() -> Option<(String, PathBuf, PathBuf)> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let key_path = home.join("eoa-backup/aes.key");
        let sav = home.join("eoa-backup/saves/SaveData.work.sav");
        let cat = home.join("projects/modding/eoa/echoes-of-aincrad-mods/docs/data");
        if key_path.exists() && sav.exists() && cat.join("weapons.tsv").exists() {
            Some((std::fs::read_to_string(key_path).ok()?, sav, cat))
        } else {
            None
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("aml-hygiene-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn real_save_scan_is_structurally_sound() {
        let Some((key, sav, cat_dir)) = local() else { return };
        let file = SaveFile::load(&sav, key.trim()).expect("load");
        let cat = Catalog::from_dir(&cat_dir).expect("catalog");
        let r = scan(file.save_tree(), &cat);
        assert!(r.entries > 0, "no chest entries found — schema drift?");
        assert!(
            r.uncatalogued.is_empty(),
            "chest entries with unmapped category enums (add to map_enum): {:?}",
            r.uncatalogued
        );
        assert!(
            r.dangling_equips().is_empty(),
            "equipment references no chest entry: {:?}",
            r.dangling_equips()
        );
    }

    #[test]
    fn injected_stale_items_are_detected() {
        let Some((key, sav, cat_dir)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let cat = Catalog::from_dir(&cat_dir).expect("catalog");
        let injected = inject_test_items(file.save_tree_mut());
        assert_eq!(injected.len(), 2, "expected weapon + recipe injection: {injected:?}");
        let r = scan(file.save_tree(), &cat);
        assert!(r.stale.iter().any(|s| s.id == 999 && s.unique_id == Some(999_999)));
        assert!(r.stale.iter().any(|s| s.category == "Dagger" && s.id == 38));
    }

    #[test]
    fn purge_round_trips_byte_identical() {
        let Some((key, sav, cat_dir)) = local() else { return };
        let cat = Catalog::from_dir(&cat_dir).expect("catalog");

        // Baseline: load + write untouched (normalizes through our writer).
        let base = SaveFile::load(&sav, key.trim()).expect("load");
        let base_out = tmp("base.sav");
        base.write(&base_out).expect("write base");

        // Inject stale items, purge them, write.
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        inject_test_items(file.save_tree_mut());
        let removed = purge(file.save_tree_mut(), &cat).expect("purge not blocked");
        assert_eq!(removed.len(), 2, "exactly the injected entries: {removed:?}");
        let clean_out = tmp("clean.sav");
        file.write(&clean_out).expect("write clean");

        let a = std::fs::read(&base_out).unwrap();
        let b = std::fs::read(&clean_out).unwrap();
        assert_eq!(a, b, "inject→purge must round-trip byte-identical");
        let _ = std::fs::remove_file(base_out);
        let _ = std::fs::remove_file(clean_out);
    }

    #[test]
    fn purge_refuses_when_stale_item_is_equipped() {
        let Some((key, sav, cat_dir)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let cat = Catalog::from_dir(&cat_dir).expect("catalog");
        inject_test_items(file.save_tree_mut());
        // Point slot-0's equipped weapon at the injected stale item.
        set_first_equipped_weapon(&mut file.save_tree_mut().root.properties, 999_999);
        let err = purge(file.save_tree_mut(), &cat).expect_err("must refuse");
        assert!(err.0.iter().any(|(path, uid)| *uid == 999_999 && path.contains("Weapon")));
        // And the save must be unmutated: the stale items are still there.
        let r = scan(file.save_tree(), &cat);
        assert_eq!(r.stale.len(), 2, "refusal must not partially purge");
    }

    /// Test helper: set the first Equipment.WeaponUniqueID in the tree.
    fn set_first_equipped_weapon(props: &mut Properties, uid: i32) -> bool {
        for (k, p) in props.0.iter_mut() {
            match p {
                Property::Struct(StructValue::Struct(inner)) => {
                    if k.1 == "Equipment" {
                        for (ek, ep) in inner.0.iter_mut() {
                            if ek.1 == "WeaponUniqueID" {
                                if let Property::Int(v) = ep {
                                    *v = uid;
                                    return true;
                                }
                            }
                        }
                    }
                    if set_first_equipped_weapon(inner, uid) {
                        return true;
                    }
                }
                Property::Array(ValueVec::Struct(v)) => {
                    for sv in v.iter_mut() {
                        if let StructValue::Struct(inner) = sv {
                            if set_first_equipped_weapon(inner, uid) {
                                return true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }
}
