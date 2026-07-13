//! NPC-hair picker that drives the `hairswap` UE4SS mod.
//!
//! The player's own hair is edited directly in the save. NPC-only hairs can't be
//! set that way — the game resolves them through the native AssetManager, which
//! nulls a redirected part and crashes — so instead we write the chosen id to the
//! `hairswap` mod's config file. The mod (UE4SS/Lua) reads it and sets the hair on
//! the player's `HeadGearMesh` component at runtime. Requires UE4SS + hairswap.

use std::path::{Path, PathBuf};

/// The 27 NPC-only hair style ids — the full `DT_HeadGearParts` NPC set, kept in
/// sync with `hairswap`'s id list and `main.rs`'s `NPC_HAIR`.
pub const NPC_HAIR_IDS: &[u32] = &[
    800001, 801001, 801021, 802001, 803001, 804001, 805001, 806001, 807001, 807031,
    807502, 807504, 808001, 809001, 850001, 850505, 851001, 851503, 852001, 852011,
    853001, 854001, 854011, 855001, 856001, 856031, 857001,
];

/// Path to the hairswap mod's config file, if the mod is installed in the detected
/// game: `<game>/<Project>/Binaries/Win64/ue4ss/Mods/hairswap/config.txt`.
/// `None` when the game or the mod folder isn't present. Called once at startup
/// (it runs Steam-library detection), not per frame.
pub fn config_path() -> Option<PathBuf> {
    let game = aml_host::find_game().ok()?;
    let dir = game.layout.ue4ss_mods_dir().join("hairswap");
    dir.is_dir().then(|| dir.join("config.txt"))
}

/// The currently-configured NPC hair id (None if unset / absent / unparsable / 0).
pub fn read(path: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(path).ok()?;
    s.split(|c: char| !c.is_ascii_digit())
        .find(|t| !t.is_empty())
        .and_then(|t| t.parse().ok())
        .filter(|id| *id != 0)
}

/// Write the chosen hair id, or clear the override (`None` writes `0`, which the
/// mod treats as "no override — use my real hair").
pub fn write(path: &Path, id: Option<u32>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", id.unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("aml-npchair-{name}.txt"))
    }

    #[test]
    fn write_then_read_round_trips() {
        let p = tmp("roundtrip");
        write(&p, Some(854011)).unwrap();
        assert_eq!(read(&p), Some(854011));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn clear_writes_zero_and_reads_none() {
        let p = tmp("clear");
        write(&p, None).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), "0");
        assert_eq!(read(&p), None);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_file_reads_none() {
        assert_eq!(read(&tmp("does-not-exist-xyz")), None);
    }

    #[test]
    fn stray_text_and_trailing_newline_tolerated() {
        let p = tmp("stray");
        std::fs::write(&p, "  807502  \n# a comment\n").unwrap();
        assert_eq!(read(&p), Some(807502));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn ids_match_mod_count() {
        // The full DT_HeadGearParts NPC set (850505 included since 0.1.12).
        assert_eq!(NPC_HAIR_IDS.len(), 27);
    }
}
