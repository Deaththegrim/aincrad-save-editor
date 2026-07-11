#[test]
#[ignore] // run explicitly: needs the game running
fn recover_from_live_game() {
    let pak = std::path::Path::new(
        "/home/junie/.local/share/Steam/steamapps/common/Echoes of Aincrad/EchoesofAincrad/Content/Paks/pakchunk0-WindowsClient.pak");
    match aml_keyscan::recover_key(pak) {
        Ok(key) => { eprintln!("RECOVERED: {key}"); }
        Err(e) => { eprintln!("recovery error: {e}"); }
    }
}
