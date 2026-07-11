#[test]
fn known_key_validates_against_real_pak() {
    let pak = std::path::Path::new(
        "/home/junie/.local/share/Steam/steamapps/common/Echoes of Aincrad/EchoesofAincrad/Content/Paks/pakchunk0-WindowsClient.pak");
    if !pak.exists() { return; }
    let enc = aml_keyscan::pak_index_block(pak).expect("pak index block");
    // the real key (from local backup) must validate; a wrong key must not.
    let key_hex = std::fs::read_to_string(format!("{}/eoa-backup/aes.key", std::env::var("HOME").unwrap()))
        .unwrap_or_default();
    let key_hex = key_hex.trim().trim_start_matches("0x");
    if key_hex.len() != 64 { return; }
    let mut key = [0u8; 32];
    for i in 0..32 { key[i] = u8::from_str_radix(&key_hex[i*2..i*2+2], 16).unwrap(); }
    assert!(aml_keyscan::key_decrypts_pak(&key, &enc), "real key must validate the pak index");
    let wrong = [0u8; 32];
    assert!(!aml_keyscan::key_decrypts_pak(&wrong, &enc), "zero key must not validate");
}
