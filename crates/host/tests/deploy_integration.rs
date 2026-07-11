//! End-to-end deploy against a fake install tree in a tempdir.
//!
//! Proves the real (non-dry-run) executor: a staged pak mod + a Lua mod land in
//! the right places with the right names, and `mods.txt` is written. This is the
//! path that actually matters tomorrow — dry-run alone wouldn't catch a broken
//! copy or a wrong destination.

use aml_core::layout::InstallLayout;
use aml_core::order::resolve_order;
use aml_core::plan::{plan_deploy, plan_reconcile, DeployPlan};
use aml_host::deploy::{execute_plan, load_manifest, save_manifest};
use aml_host::ModLibrary;
use std::fs;
use std::path::Path;

/// Full deploy with reconciliation, mirroring what `aml deploy --apply` does.
fn deploy(library: &ModLibrary, layout: &InstallLayout) {
    let profile = library.load_profile().unwrap();
    let order = resolve_order(&profile).unwrap();
    let plan = plan_deploy(&order, layout).unwrap();
    let prev = load_manifest(layout);
    let mut ops = plan_reconcile(&prev, &plan.manifest, layout);
    ops.extend(plan.ops.clone());
    let full = DeployPlan {
        ops,
        manifest: plan.manifest.clone(),
    };
    execute_plan(&full, library, false).unwrap();
    save_manifest(layout, &plan.manifest).unwrap();
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn deploys_pak_and_lua_mods_into_install() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // --- fake mod library --------------------------------------------------
    let lib_root = base.join("library");
    let state = base.join("profile.json");
    let library = ModLibrary {
        root: lib_root.clone(),
        state_path: state,
    };

    // A pak mod: mod.json + a payload .pak.
    write(
        &lib_root.join("author.swords/mod.json"),
        r#"{"id":"author.swords","name":"Swords","version":"1.0.0","kind":"pak","priority":100,"requires":["base.core"]}"#,
    );
    write(
        &lib_root.join("author.swords/swords.pak"),
        "FAKE-PAK-BYTES",
    );

    // A Lua mod: mod.json + scripts/main.lua (loads first via priority 0).
    write(
        &lib_root.join("base.core/mod.json"),
        r#"{"id":"base.core","name":"Core","version":"1.0.0","kind":"lua","priority":0}"#,
    );
    write(
        &lib_root.join("base.core/scripts/main.lua"),
        "print('core loaded')",
    );

    // --- fake install ------------------------------------------------------
    let game_root = base.join("game/common/Echoes of Aincrad");
    let layout = InstallLayout::new(&game_root, "EchoesOfAincrad");
    fs::create_dir_all(layout.paks_dir()).unwrap();
    fs::create_dir_all(layout.win64_dir()).unwrap();

    // --- resolve -> plan -> execute (for real) -----------------------------
    let profile = library.load_profile().unwrap();
    let order = resolve_order(&profile).unwrap();
    // base.core (pri 0, dependency) must resolve before author.swords.
    let ids: Vec<_> = order.iter().map(|e| e.manifest.id.0.as_str()).collect();
    assert_eq!(ids, vec!["base.core", "author.swords"]);

    let plan = plan_deploy(&order, &layout).unwrap();
    let report = execute_plan(&plan, &library, false).unwrap();
    assert!(!report.dry_run);

    // --- assertions on the resulting tree ----------------------------------
    // Pak landed in ~mods with the resolved order prefix (index 1 => 001).
    let deployed_pak = layout.mods_dir().join("001_author.swords_P.pak");
    assert!(deployed_pak.is_file(), "pak not deployed: {}", deployed_pak.display());
    assert_eq!(fs::read_to_string(&deployed_pak).unwrap(), "FAKE-PAK-BYTES");

    // Lua mod folder copied into ue4ss/Mods, with its script, WITHOUT mod.json.
    // Deployed UE4SS folder is dot-free (dots break UE4SS mod loading).
    let lua_main = layout.ue4ss_mods_dir().join("base_core/scripts/main.lua");
    assert!(lua_main.is_file(), "lua script not deployed: {}", lua_main.display());
    assert!(
        !layout.ue4ss_mods_dir().join("base_core/mod.json").exists(),
        "library-only mod.json should not be deployed into the game"
    );

    // mods.txt registers the Lua mod (dot-free name) as enabled.
    let mods_txt = fs::read_to_string(layout.ue4ss_mods_txt()).unwrap();
    assert!(mods_txt.contains("base_core : 1"), "mods.txt: {mods_txt:?}");
}

#[test]
fn reconcile_removes_files_and_registry_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let library = ModLibrary {
        root: base.join("library"),
        state_path: base.join("profile.json"),
    };
    // A pak mod + a lua mod, both enabled.
    write(
        &library.root.join("a.pak_mod/mod.json"),
        r#"{"id":"a.pak_mod","name":"A","version":"1","kind":"pak"}"#,
    );
    write(&library.root.join("a.pak_mod/a.pak"), "BYTES");
    write(
        &library.root.join("z.lua_mod/mod.json"),
        r#"{"id":"z.lua_mod","name":"Z","version":"1","kind":"lua"}"#,
    );
    write(&library.root.join("z.lua_mod/Scripts/main.lua"), "print('z')");

    let layout = InstallLayout::new(base.join("game"), "Proj");
    fs::create_dir_all(layout.paks_dir()).unwrap();
    // Seed a UE4SS mods.txt/json with a built-in so we can prove it survives.
    fs::create_dir_all(layout.ue4ss_mods_dir()).unwrap();
    fs::write(layout.ue4ss_mods_txt(), "BuiltIn : 1\nKeybinds : 1\n").unwrap();
    fs::write(
        layout.ue4ss_mods_dir().join("mods.json"),
        r#"[{"mod_name":"BuiltIn","mod_enabled":true},{"mod_name":"Keybinds","mod_enabled":true}]"#,
    )
    .unwrap();

    // First deploy: both mods land + get registered.
    deploy(&library, &layout);
    let lua_dir = layout.ue4ss_mods_dir().join("z_lua_mod");
    let pak = layout.mods_dir().join("000_a.pak_mod_P.pak");
    assert!(lua_dir.is_dir(), "lua mod should be deployed");
    assert!(pak.is_file(), "pak should be deployed");
    assert!(fs::read_to_string(layout.ue4ss_mods_txt()).unwrap().contains("z_lua_mod : 1"));

    // Disable the lua mod, redeploy → its files + registry entries are removed,
    // the pak stays, and the built-in is untouched.
    library
        .set_enabled(&aml_core::manifest::ModId("z.lua_mod".into()), false)
        .unwrap();
    deploy(&library, &layout);

    assert!(!lua_dir.exists(), "disabled lua mod dir should be removed");
    assert!(pak.is_file(), "enabled pak should remain");
    let txt = fs::read_to_string(layout.ue4ss_mods_txt()).unwrap();
    assert!(!txt.contains("z_lua_mod"), "lua mod should be unregistered from mods.txt");
    assert!(txt.contains("BuiltIn : 1"), "built-in must survive: {txt:?}");
    let json = fs::read_to_string(layout.ue4ss_mods_dir().join("mods.json")).unwrap();
    assert!(!json.contains("z_lua_mod"), "lua mod should be unregistered from mods.json");
    assert!(json.contains("BuiltIn"), "built-in must survive in json");
}

#[test]
fn disabled_mod_is_excluded_from_deploy() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let library = ModLibrary {
        root: base.join("library"),
        state_path: base.join("profile.json"),
    };
    write(
        &library.root.join("a.pak_mod/mod.json"),
        r#"{"id":"a.pak_mod","name":"A","version":"1","kind":"pak"}"#,
    );
    write(&library.root.join("a.pak_mod/a.pak"), "BYTES");

    // Disable it through the persisted-state API.
    library
        .set_enabled(&aml_core::manifest::ModId("a.pak_mod".into()), false)
        .unwrap();

    let layout = InstallLayout::new(base.join("game"), "Proj");
    fs::create_dir_all(layout.paks_dir()).unwrap();

    let profile = library.load_profile().unwrap();
    let order = resolve_order(&profile).unwrap();
    assert!(order.is_empty(), "disabled mod should not be in load order");

    let plan = plan_deploy(&order, &layout).unwrap();
    execute_plan(&plan, &library, false).unwrap();
    // ~mods dir exists (EnsureDir) but holds no pak.
    let count = fs::read_dir(layout.mods_dir())
        .map(|rd| rd.count())
        .unwrap_or(0);
    assert_eq!(count, 0, "no paks should be deployed");
}
