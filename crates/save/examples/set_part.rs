//! Scriptable appearance setter (used by eyeforge's eyetool `select` step).
//!
//! Usage: cargo run -p aml-save --example set_part -- <save-path> <slot> <field> <int-value>
//!    or: ... <save-path> <slot> <field> <r> <g> <b> <a>   (color fields, linear floats)
//! e.g.:  cargo run -p aml-save --example set_part -- ~/.../SaveData.sav 0 Pupil 20
//! Reads the key from ~/eoa-backup/aes.key. Writes the save IN PLACE — back it
//! up first (eyetool does). Prints old -> new on success.

use aml_save::appearance::FieldValue;
use aml_save::SaveFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let (path, slot, field, value) = match (args.next(), args.next(), args.next(), args.next()) {
        (Some(p), Some(s), Some(f), Some(v)) => (p, s, f, v),
        _ => {
            eprintln!("usage: set_part <save-path> <slot> <field> <int-value>");
            std::process::exit(2);
        }
    };
    let slot: usize = slot.parse().expect("slot must be an integer");
    let extra: Vec<String> = args.collect();
    let home = std::env::var("HOME").unwrap();
    let key = std::fs::read_to_string(format!("{home}/eoa-backup/aes.key")).unwrap();

    let mut sf = SaveFile::load(&path, key.trim()).expect("load/decrypt failed");
    let old = sf
        .appearance(slot)
        .ok()
        .and_then(|fs| fs.into_iter().find(|f| f.name == field))
        .map(|f| format!("{:?}", f.value))
        .unwrap_or_else(|| "?".into());
    let fv = if extra.len() == 3 {
        let g: f32 = extra[0].parse().unwrap();
        let b: f32 = extra[1].parse().unwrap();
        let a: f32 = extra[2].parse().unwrap();
        FieldValue::Color([value.parse::<f32>().expect("r float"), g, b, a])
    } else {
        FieldValue::Int(value.parse::<i32>().expect("value must be an integer"))
    };
    sf.set_appearance(slot, &field, fv.clone())
        .expect("set_appearance failed");
    sf.write(&path).expect("write failed");
    println!("{field}[slot {slot}]: {old} -> {fv:?}");
}
