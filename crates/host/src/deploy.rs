//! Execute a core `DeployPlan` against the real filesystem.
//!
//! Core decides *what* lands *where* (a pure `DeployPlan`); this carries it out,
//! resolving each mod's payload from the [`ModLibrary`]. `dry_run` records
//! intended actions without touching disk.

use crate::library::{copy_dir_recursive, ModLibrary};
use crate::HostError;
use aml_core::layout::InstallLayout;
use aml_core::plan::{DeployManifest, DeployPlan, FileOp};

/// Result of executing (or dry-running) a plan.
#[derive(Debug, Default)]
pub struct DeployReport {
    pub actions: Vec<String>,
    pub dry_run: bool,
}

/// Execute a deploy plan.
pub fn execute_plan(
    plan: &DeployPlan,
    library: &ModLibrary,
    dry_run: bool,
) -> Result<DeployReport, HostError> {
    let mut report = DeployReport {
        dry_run,
        ..Default::default()
    };

    for op in &plan.ops {
        match op {
            FileOp::EnsureDir { path } => {
                report.actions.push(format!("mkdir -p {}", path.display()));
                if !dry_run {
                    std::fs::create_dir_all(path)?;
                }
            }
            FileOp::DeployPak { mod_id, dest, .. } => {
                let src = library
                    .find_pak(mod_id)
                    .ok_or_else(|| HostError::Other(format!("no .pak staged for mod '{mod_id}'")))?;
                report
                    .actions
                    .push(format!("copy {} -> {}", src.display(), dest.display()));
                if !dry_run {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(&src, dest)?;
                }
            }
            FileOp::DeployUe4ssMod { mod_id, dest_dir } => {
                let src = library.mod_dir(mod_id);
                report
                    .actions
                    .push(format!("copy dir {} -> {}", src.display(), dest_dir.display()));
                if !dry_run {
                    // Don't ship the library-only manifest into the game.
                    copy_dir_recursive(&src, dest_dir, &["mod.json"])?;
                }
            }
            FileOp::RegisterUe4ssMods { path, names } => {
                let existing = std::fs::read_to_string(path).unwrap_or_default();
                let merged = merge_mods_txt(&existing, names);
                report.actions.push(format!(
                    "merge {} into {} (+{} mod{})",
                    names.join(", "),
                    path.display(),
                    names.len(),
                    if names.len() == 1 { "" } else { "s" }
                ));
                if !dry_run {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, merged)?;
                }

                // Newer UE4SS builds keep a mods.json alongside mods.txt and the
                // pair must stay in sync — a mod present only in mods.txt does
                // NOT start (verified against the EoA build). Merge it too.
                let json_path = path.with_file_name("mods.json");
                if json_path.is_file() {
                    let merged_json = merge_mods_json(
                        &std::fs::read_to_string(&json_path).unwrap_or_default(),
                        names,
                    )?;
                    report
                        .actions
                        .push(format!("merge {} into {}", names.join(", "), json_path.display()));
                    if !dry_run {
                        std::fs::write(&json_path, merged_json)?;
                    }
                }
            }
            FileOp::RemovePath { path } => {
                if path.is_dir() {
                    report.actions.push(format!("rm -r {}", path.display()));
                    if !dry_run {
                        std::fs::remove_dir_all(path)?;
                    }
                } else if path.exists() {
                    report.actions.push(format!("rm {}", path.display()));
                    if !dry_run {
                        std::fs::remove_file(path)?;
                    }
                }
            }
            FileOp::UnregisterUe4ssMods { path, names } => {
                report
                    .actions
                    .push(format!("unregister {} from {}", names.join(", "), path.display()));
                if !dry_run {
                    if let Ok(existing) = std::fs::read_to_string(path) {
                        std::fs::write(path, remove_from_mods_txt(&existing, names))?;
                    }
                    let json_path = path.with_file_name("mods.json");
                    if let Ok(existing) = std::fs::read_to_string(&json_path) {
                        std::fs::write(&json_path, remove_from_mods_json(&existing, names)?)?;
                    }
                }
            }
        }
    }

    Ok(report)
}

/// Load aml's deploy manifest for an install (empty if none yet / unreadable).
pub fn load_manifest(layout: &InstallLayout) -> DeployManifest {
    std::fs::read_to_string(layout.deploy_manifest())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist aml's deploy manifest for an install.
pub fn save_manifest(layout: &InstallLayout, manifest: &DeployManifest) -> Result<(), HostError> {
    let path = layout.deploy_manifest();
    let text = serde_json::to_string_pretty(manifest).map_err(|e| HostError::Other(e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Result of `verify`: which deployed mods are intact vs. missing files.
#[derive(Debug, Default)]
pub struct VerifyReport {
    pub ok: Vec<String>,
    /// (mod id, missing path) pairs.
    pub missing: Vec<(String, String)>,
}

impl VerifyReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Check that every file aml recorded deploying still exists in the install.
/// Catches a mod folder deleted by hand, a failed copy, or a game update that
/// wiped `~mods`.
pub fn verify(layout: &InstallLayout) -> VerifyReport {
    let manifest = load_manifest(layout);
    let mut report = VerifyReport::default();
    for m in &manifest.mods {
        let mut intact = true;
        for p in &m.paths {
            if !p.exists() {
                report.missing.push((m.id.0.clone(), p.display().to_string()));
                intact = false;
            }
        }
        if intact {
            report.ok.push(m.id.0.clone());
        }
    }
    report
}

/// Parse a mods.txt line into its mod name, if it is a `Name : flag` entry.
fn mods_txt_entry_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with(';') {
        return None;
    }
    trimmed.split_once(':').map(|(name, _)| name.trim())
}

/// Upsert `names` into a UE4SS mods.txt, preserving all existing lines
/// (built-in mods, comments like "; do not move up!", blank lines) and their
/// order. Existing entries are set enabled; new names are inserted after the
/// last regular entry but before the trailing "Keybinds" block (which UE4SS
/// requires to stay last).
fn merge_mods_txt(existing: &str, names: &[String]) -> String {
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();

    for name in names {
        // Re-enable an existing entry in place.
        if let Some(line) = lines
            .iter_mut()
            .find(|l| mods_txt_entry_name(l) == Some(name.as_str()))
        {
            *line = format!("{name} : 1");
            continue;
        }
        // Otherwise insert after the last non-Keybinds entry (before the tail
        // block of blanks/comment/Keybinds).
        let insert_at = lines
            .iter()
            .rposition(|l| matches!(mods_txt_entry_name(l), Some(n) if n != "Keybinds"))
            .map(|i| i + 1)
            .unwrap_or(lines.len());
        lines.insert(insert_at, format!("{name} : 1"));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Drop entries for `names` from a mods.txt, leaving everything else intact.
fn remove_from_mods_txt(existing: &str, names: &[String]) -> String {
    let mut out: String = existing
        .lines()
        .filter(|l| !matches!(mods_txt_entry_name(l), Some(n) if names.iter().any(|x| x == n)))
        .collect::<Vec<_>>()
        .join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Drop entries for `names` from a mods.json array.
fn remove_from_mods_json(existing: &str, names: &[String]) -> Result<String, HostError> {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Entry {
        mod_name: String,
        mod_enabled: bool,
    }
    let mut entries: Vec<Entry> = if existing.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(existing).map_err(|e| HostError::Other(format!("mods.json: {e}")))?
    };
    entries.retain(|e| !names.contains(&e.mod_name));
    serde_json::to_string_pretty(&entries).map_err(|e| HostError::Other(e.to_string()))
}

/// Upsert `names` (enabled) into a UE4SS mods.json array, preserving existing
/// entries and their order. New names are inserted before the "Keybinds" entry
/// (which UE4SS ships last), mirroring the mods.txt convention.
fn merge_mods_json(existing: &str, names: &[String]) -> Result<String, HostError> {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Entry {
        mod_name: String,
        mod_enabled: bool,
    }

    let mut entries: Vec<Entry> = if existing.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(existing)
            .map_err(|e| HostError::Other(format!("mods.json parse: {e}")))?
    };

    for name in names {
        if let Some(e) = entries.iter_mut().find(|e| e.mod_name == *name) {
            e.mod_enabled = true;
        } else {
            // Insert before a trailing "Keybinds" entry if present.
            let idx = entries
                .iter()
                .position(|e| e.mod_name == "Keybinds")
                .unwrap_or(entries.len());
            entries.insert(
                idx,
                Entry {
                    mod_name: name.clone(),
                    mod_enabled: true,
                },
            );
        }
    }

    serde_json::to_string_pretty(&entries).map_err(|e| HostError::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aml_core::layout::InstallLayout;
    use aml_core::manifest::{ModKind, ModManifest, Profile, ProfileEntry};
    use aml_core::order::resolve_order;
    use aml_core::plan::plan_deploy;
    use std::path::PathBuf;

    #[test]
    fn merge_preserves_builtins_and_comments() {
        let existing = "CheatManagerEnablerMod : 1\n\
                        ConsoleEnablerMod : 1\n\
                        SplitScreenMod : 0\n\
                        \n\
                        ; Built-in keybinds, do not move up!\n\
                        Keybinds : 1\n";
        let merged = merge_mods_txt(existing, &["aml.hello".to_string()]);
        // Built-ins and the comment survive untouched.
        assert!(merged.contains("CheatManagerEnablerMod : 1"));
        assert!(merged.contains("ConsoleEnablerMod : 1"));
        assert!(merged.contains("SplitScreenMod : 0")); // a disabled one stays disabled
        assert!(merged.contains("; Built-in keybinds, do not move up!"));
        assert!(merged.contains("Keybinds : 1"));
        // Our mod is appended and enabled.
        assert!(merged.contains("aml.hello : 1"));
    }

    #[test]
    fn merge_reenables_existing_entry_not_duplicate() {
        let existing = "aml.hello : 0\nKeybinds : 1\n";
        let merged = merge_mods_txt(existing, &["aml.hello".to_string()]);
        assert!(merged.contains("aml.hello : 1"));
        assert!(!merged.contains("aml.hello : 0"));
        // No duplicate line.
        assert_eq!(merged.matches("aml.hello").count(), 1);
    }

    #[test]
    fn merge_inserts_before_keybinds_block() {
        let existing = "BPModLoaderMod : 1\n\n; do not move up!\nKeybinds : 1\n";
        let merged = merge_mods_txt(existing, &["mymod".to_string()]);
        let bp = merged.find("BPModLoaderMod").unwrap();
        let mine = merged.find("mymod : 1").unwrap();
        let keys = merged.find("Keybinds : 1").unwrap();
        // Ours lands after the last real entry but before Keybinds.
        assert!(bp < mine && mine < keys, "order wrong:\n{merged}");
    }

    #[test]
    fn remove_from_txt_drops_only_named() {
        let existing = "A : 1\nmymod : 1\nKeybinds : 1\n";
        let out = remove_from_mods_txt(existing, &["mymod".to_string()]);
        assert!(!out.contains("mymod"));
        assert!(out.contains("A : 1"));
        assert!(out.contains("Keybinds : 1"));
    }

    #[test]
    fn remove_from_json_drops_only_named() {
        let existing = r#"[{"mod_name":"A","mod_enabled":true},{"mod_name":"mymod","mod_enabled":true}]"#;
        let out = remove_from_mods_json(existing, &["mymod".to_string()]).unwrap();
        assert!(out.contains("\"A\""));
        assert!(!out.contains("mymod"));
    }

    #[test]
    fn dry_run_reports_without_writing() {
        let entry = ProfileEntry {
            manifest: ModManifest {
                id: "a.lua".into(),
                name: "lua".into(),
                version: "1".into(),
                kind: ModKind::Lua,
                priority: 0,
                requires: vec![],
                author: None,
                description: None,
            },
            enabled: true,
            priority_override: None,
        };
        let profile = Profile {
            name: "t".into(),
            mods: vec![entry],
        };
        let order = resolve_order(&profile).unwrap();
        let layout = InstallLayout::new("/games/EoA", "EchoesOfAincrad");
        let plan = plan_deploy(&order, &layout).unwrap();

        let library = ModLibrary {
            root: PathBuf::from("/nonexistent/library"),
            state_path: PathBuf::from("/nonexistent/profile.json"),
        };
        let report = execute_plan(&plan, &library, true).unwrap();
        assert!(report.dry_run);
        assert!(report.actions.iter().any(|a| a.contains("copy dir")));
        assert!(report.actions.iter().any(|a| a.contains("mkdir")));
    }
}
