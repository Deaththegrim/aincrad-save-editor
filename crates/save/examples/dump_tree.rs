//! Dev tool: dump the property tree of a local save (names/types, values
//! elided; strings shown) to survey what the editor doesn't expose yet.
//!
//! Usage: cargo run -p aml-save --example dump_tree [depth] [save-path]
//! Reads ~/eoa-backup/aes.key + ~/eoa-backup/saves/SaveData.work.sav (or [save-path]).

use uesave::{Properties, Property, Save, StructValue, ValueVec};

fn main() {
    let home = std::env::var("HOME").unwrap();
    let depth: usize =
        std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    let save_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| format!("{home}/eoa-backup/saves/SaveData.work.sav"));
    let key_hex =
        std::fs::read_to_string(format!("{home}/eoa-backup/aes.key")).unwrap();
    let raw = std::fs::read(save_path).unwrap();
    let key = aml_save::crypto::parse_key(key_hex.trim()).unwrap();
    let plain = aml_save::crypto::decrypt(&key, &raw).unwrap();
    let save = Save::read(&mut std::io::Cursor::new(&plain)).unwrap();
    walk(&save.root.properties, 0, depth);
}

fn walk(props: &Properties, indent: usize, depth: usize) {
    for (k, p) in props.0.iter() {
        line(&k.1, p, indent, depth);
    }
}

fn line(name: &str, p: &Property, indent: usize, depth: usize) {
    let pad = "  ".repeat(indent);
    match p {
        Property::Struct(StructValue::Struct(inner)) => {
            println!("{pad}{name}: Struct ({} fields)", inner.0.len());
            if indent < depth {
                walk(inner, indent + 1, depth);
            }
        }
        Property::Struct(sv) => println!("{pad}{name}: Struct::{}", variant(sv)),
        Property::Array(ValueVec::Struct(v)) => {
            println!("{pad}{name}: Array<Struct> len {}", v.len());
            if indent < depth {
                if let Some(StructValue::Struct(inner)) = v.first() {
                    walk(inner, indent + 1, depth);
                }
            }
        }
        Property::Array(v) => println!("{pad}{name}: Array ({})", vec_desc(v)),
        Property::Map(m) => println!("{pad}{name}: Map len {}", m.len()),
        Property::Str(s) => println!("{pad}{name}: Str {s:?}"),
        Property::Name(s) => println!("{pad}{name}: Name {s}"),
        Property::Enum(s) => println!("{pad}{name}: Enum {s}"),
        Property::Int(v) => println!("{pad}{name}: Int {v}"),
        Property::Int64(v) => println!("{pad}{name}: Int64 {v}"),
        Property::UInt32(v) => println!("{pad}{name}: UInt32 {v}"),
        Property::Float(v) => println!("{pad}{name}: Float {}", v.0),
        Property::Double(v) => println!("{pad}{name}: Double {}", v.0),
        Property::Bool(v) => println!("{pad}{name}: Bool {v}"),
        Property::Byte(b) => println!("{pad}{name}: Byte {b:?}"),
        other => println!("{pad}{name}: {}", short_debug(other)),
    }
}

fn variant(sv: &StructValue) -> String {
    short_debug(sv)
}

fn vec_desc(v: &ValueVec) -> String {
    short_debug(v)
}

/// First token of the Debug output — enough to name the variant without values.
fn short_debug(d: &impl std::fmt::Debug) -> String {
    let s = format!("{d:?}");
    s.split(['(', '{', ' ']).next().unwrap_or("?").to_string()
}
