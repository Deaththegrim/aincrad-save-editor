//! Print the equipped weapon's chest entry (ItemId/Category) by joining
//! Equipment.WeaponUniqueID against WeaponChest. Read-only.
//!
//! Usage: cargo run -p aml-save --example get_equipment -- <save-path>

use uesave::{Properties, Property, Save, StructValue, ValueVec};

fn get<'a>(props: &'a Properties, name: &str) -> Option<&'a Property> {
    props.0.iter().find(|(k, _)| k.1 == name).map(|(_, p)| p)
}

/// depth-first search for a named property anywhere in the tree
fn find<'a>(props: &'a Properties, name: &str) -> Option<&'a Property> {
    if let Some(p) = get(props, name) {
        return Some(p);
    }
    for (_, p) in props.0.iter() {
        match p {
            Property::Struct(StructValue::Struct(inner)) => {
                if let Some(f) = find(inner, name) {
                    return Some(f);
                }
            }
            Property::Array(ValueVec::Struct(v)) => {
                for sv in v {
                    if let StructValue::Struct(inner) = sv {
                        if let Some(f) = find(inner, name) {
                            return Some(f);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn int(props: &Properties, name: &str) -> Option<i32> {
    match get(props, name) {
        Some(Property::Int(v)) => Some(*v),
        _ => None,
    }
}

fn main() {
    let home = std::env::var("HOME").unwrap();
    let save_path = std::env::args().nth(1).expect("usage: get_equipment <save>");
    let key_hex = std::fs::read_to_string(format!("{home}/eoa-backup/aes.key")).unwrap();
    let key = aml_save::crypto::parse_key(key_hex.trim()).unwrap();
    let plain = aml_save::crypto::decrypt(&key, &std::fs::read(save_path).unwrap()).unwrap();
    let save = Save::read(&mut std::io::Cursor::new(&plain)).unwrap();
    let root = &save.root.properties;

    let equip = match find(root, "Equipment") {
        Some(Property::Struct(StructValue::Struct(inner))) => inner,
        _ => panic!("no Equipment struct"),
    };
    let wid = int(equip, "WeaponUniqueID").expect("no WeaponUniqueID");
    println!("Equipment.WeaponUniqueID = {wid}");

    let chest = match find(root, "WeaponChest") {
        Some(Property::Array(ValueVec::Struct(v))) => v,
        _ => panic!("no WeaponChest"),
    };
    for sv in chest {
        if let StructValue::Struct(inner) = sv {
            if int(inner, "UniqueID") == Some(wid) {
                for (k, p) in inner.0.iter() {
                    match p {
                        Property::Int(v) => println!("  {} = {v}", k.1),
                        Property::Enum(s) => println!("  {} = {s}", k.1),
                        Property::Byte(b) => println!("  {} = {b:?}", k.1),
                        _ => {}
                    }
                }
                return;
            }
        }
    }
    println!("unique id {wid} not found in chest");
}
