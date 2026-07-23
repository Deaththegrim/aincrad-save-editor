//! Dev CLI over [`aml_save::hygiene`]: find (and optionally purge) modded/
//! stale item ids that outlive a mod uninstall. Read-only by default.
//!
//! Usage:
//!   cargo run -p aml-save --example scan_items <save> <catalog-dir>
//!   cargo run -p aml-save --example scan_items <save> <catalog-dir> --inject <out>
//!   cargo run -p aml-save --example scan_items <save> <catalog-dir> --purge <out>
//!
//! <catalog-dir> = echoes-of-aincrad-mods/docs/data. --inject plants a
//! synthetic stale weapon + recipe id into a COPY (self-test). Never point
//! --inject or --purge at the live save.

use aml_save::hygiene::{self, Catalog, Report};

fn print_report(r: &Report) {
    println!("scanned {} chest entries", r.entries);
    println!("\n== STALE / MODDED ids (not in base catalogs): {}", r.stale.len());
    for s in &r.stale {
        println!("  {s}");
    }
    println!("\n== uncatalogued categories (enum unknown to scanner): {}", r.uncatalogued.len());
    for s in &r.uncatalogued {
        println!("  {s}");
    }
    let dangling = r.dangling_equips();
    println!("\n== equipped UniqueID refs: {} ({} dangling)", r.equip_refs.len(), dangling.len());
    for (path, uid) in dangling {
        println!("  DANGLING {path} -> UniqueID {uid}");
    }
}

fn main() {
    let home = std::env::var("HOME").unwrap();
    let args: Vec<String> = std::env::args().collect();
    let (Some(save_path), Some(cat_dir)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: scan_items <save> <catalog-dir> [--inject <out> | --purge <out>]");
        std::process::exit(2);
    };
    let mode = args.get(3).map(String::as_str);
    let out = args.get(4);
    let cat = Catalog::from_dir(cat_dir).unwrap();
    let key_hex = std::fs::read_to_string(format!("{home}/eoa-backup/aes.key")).unwrap();
    let mut sf = aml_save::SaveFile::load(save_path, key_hex.trim()).unwrap();

    match (mode, out) {
        (None, _) => print_report(&hygiene::scan(sf.save_tree(), &cat)),
        (Some("--inject"), Some(out)) => {
            for s in hygiene::inject_test_items(sf.save_tree_mut()) {
                println!("injected: {s}");
            }
            sf.write(out).unwrap();
            println!("wrote {out}");
        }
        (Some("--purge"), Some(out)) => {
            print_report(&hygiene::scan(sf.save_tree(), &cat));
            match hygiene::purge(sf.save_tree_mut(), &cat) {
                Ok(removed) if removed.is_empty() => println!("\nnothing to purge"),
                Ok(removed) => {
                    println!("\n== purged {} entries:", removed.len());
                    for s in &removed {
                        println!("  {s}");
                    }
                    sf.write(out).unwrap();
                    println!("wrote {out}");
                }
                Err(blocked) => {
                    eprintln!("ABORT: {blocked}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: scan_items <save> <catalog-dir> [--inject <out> | --purge <out>]");
            std::process::exit(2);
        }
    }
}
