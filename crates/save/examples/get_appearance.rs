//! Read-only dump of a slot's appearance fields (eyetool uses this to find
//! which FACE_S0NN texture / part ids a character wears).
//!
//! Usage: cargo run -p aml-save --example get_appearance -- <save-path> [slot]

use aml_save::SaveFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: get_appearance <save-path> [slot]");
    let slot: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let home = std::env::var("HOME").unwrap();
    let key = std::fs::read_to_string(format!("{home}/eoa-backup/aes.key")).unwrap();
    let sf = SaveFile::load(&path, key.trim()).expect("load/decrypt failed");
    for f in sf.appearance(slot).expect("no such slot") {
        println!("{} = {:?}", f.name, f.value);
    }
}
