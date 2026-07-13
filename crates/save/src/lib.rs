//! aml-save — read and edit Echoes of Aincrad character saves.
//!
//! Pipeline (all proven byte-exact): AES-256-ECB decrypt with the pak key →
//! parse the UE5 GVAS SaveGame via `uesave` → edit fields in the property tree →
//! re-serialize → re-encrypt. Untouched fields round-trip identically, so edits
//! are surgical.
//!
//! Safety: [`SaveFile::write`] backs up any file it would overwrite first, and
//! the editor should default to a working copy rather than the live save.

pub mod appearance;
pub mod crypto;
pub mod preset;

use std::path::{Path, PathBuf};
use uesave::Save;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("AES key must be 32 bytes of hex")]
    BadKey,
    #[error("save length {0} is not a multiple of 16 (not an encrypted EoA save?)")]
    BadLength(usize),
    #[error(
        "This save has an unrecognized trailer, so the editor can't safely re-pad it \
         (it would rather refuse than risk corrupting your save).\n\
         Nothing was written — your save is untouched.\n\n\
         Please send this to the developer so it can be supported:\n\
         --- EoA save diagnostic ---\n\
         serialized length: {len} (needs a multiple of 16)\n\
         uesave extra ({extra_len} bytes): {extra_hex}\n\
         last 32 bytes: {tail_hex}\n\
         ---------------------------"
    )]
    Unaligned { len: usize, extra_len: usize, extra_hex: String, tail_hex: String },
    #[error("could not parse the decrypted save as a UE5 GVAS SaveGame: {0}")]
    Parse(String),
    #[error("could not re-serialize the save: {0}")]
    Serialize(String),
    #[error("save has no character in slot {0}")]
    NoSlot(usize),
    #[error("field '{0}' not found in the appearance data")]
    NoField(String),
}

/// A loaded, decrypted character save.
pub struct SaveFile {
    key: [u8; 32],
    save: Save,
    /// Path it was loaded from (for reference; `write` can target elsewhere).
    pub source: PathBuf,
}

impl SaveFile {
    /// Decrypt + parse the save at `path` using a hex AES-256 key.
    pub fn load(path: impl AsRef<Path>, hex_key: &str) -> Result<Self, SaveError> {
        let key = crypto::parse_key(hex_key)?;
        let raw = std::fs::read(&path)?;
        let plain = crypto::decrypt(&key, &raw)?;
        let save = Save::read(&mut std::io::Cursor::new(&plain))
            .map_err(|e| SaveError::Parse(e.to_string()))?;
        Ok(Self { key, save, source: path.as_ref().to_path_buf() })
    }

    /// Re-serialize + re-encrypt and write to `path`. If `path` already exists,
    /// its current contents are first backed up (see [`backup`]), never lost.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        let path = path.as_ref();
        backup(path)?;
        let mut plain = Vec::new();
        self.save.write(&mut plain).map_err(|e| SaveError::Serialize(e.to_string()))?;
        realign_eoa(&mut plain, &self.save.extra);
        // If realign couldn't recognize the trailer, refuse with a copy-pasteable
        // diagnostic (never write a corrupt/misaligned save).
        if plain.len() % 16 != 0 {
            let tail = &plain[plain.len().saturating_sub(32)..];
            return Err(SaveError::Unaligned {
                len: plain.len(),
                extra_len: self.save.extra.len(),
                extra_hex: hex(&self.save.extra),
                tail_hex: hex(tail),
            });
        }
        let enc = crypto::encrypt(&self.key, &plain)?;
        std::fs::write(path, enc)?;
        Ok(())
    }

    /// How many character slots the save holds.
    pub fn character_count(&self) -> usize {
        appearance::slots(&self.save).map(|s| s.len()).unwrap_or(0)
    }

    /// Read every editable appearance field for a character slot.
    pub fn appearance(&self, slot: usize) -> Result<Vec<appearance::Field>, SaveError> {
        appearance::read(&self.save, slot)
    }

    /// Path this save was loaded from.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Set one appearance field on a slot.
    pub fn set_appearance(
        &mut self,
        slot: usize,
        name: &str,
        value: appearance::FieldValue,
    ) -> Result<(), SaveError> {
        appearance::set(&mut self.save, slot, name, value)
    }
}

/// Re-pad an EoA save so the whole file is a multiple of 16 (AES-ECB needs it).
///
/// The game's format is `[GVAS body][zero padding][b"GVAS"]`, where the padding
/// is sized so the total length is 16-aligned. `uesave` captures the whole
/// `[padding][GVAS]` trailer as `extra`. A length-changing edit (rename, or a
/// shorter/longer voice id) shifts the body, so the original trailer no longer
/// aligns — this rebuilds it for the current body length. It's a no-op that
/// reproduces the input byte-for-byte when nothing changed the length.
///
/// If `extra` isn't the recognized `…GVAS` trailer (some other save format), we
/// leave `plain` untouched and let `encrypt` surface the misalignment.
fn realign_eoa(plain: &mut Vec<u8>, extra: &[u8]) {
    const MAGIC: &[u8] = b"GVAS";
    // Primary: uesave hands the `[alignment zero-pad][GVAS][footer?]` trailer as
    // `extra`, with the body everything before it. Preserve the footer (from the
    // `GVAS` magic onward) and rebuild the pad so the file is 16-aligned (AES-ECB).
    if let Some(g) = extra.windows(MAGIC.len()).position(|w| w == MAGIC) {
        // `ends_with` guards the truncation: if uesave ever serialized WITHOUT
        // appending `extra`, subtracting its length would cut real body bytes.
        if plain.len() >= extra.len() && plain.ends_with(extra) {
            let footer = extra[g..].to_vec();
            let body_len = plain.len() - extra.len();
            repad_eoa(plain, body_len, &footer);
            return;
        }
    }
    // Footer-less variant: some saves end in just zero padding with no trailing
    // `GVAS` magic — uesave hands that pad as an all-zeros `extra` (seen in the
    // wild as 4 zero bytes). Keep the original extra verbatim (it's all zeros,
    // so it's correct whether the game treats it as pad or as a real zero
    // field) and rebuild the alignment pad in front of it.
    if !extra.is_empty() && extra.iter().all(|&b| b == 0) && plain.ends_with(extra) {
        let body_len = plain.len() - extra.len();
        repad_eoa(plain, body_len, extra);
        return;
    }
    // Fallback: some saves don't expose the trailer as `extra` (a different game
    // build, or a uesave parse that folds it into the body) — that's the case
    // behind a mysterious "length not a multiple of 16" on write. The serialized
    // buffer still ENDS with `[…None terminator][zero-pad][GVAS]`. Anchor on the
    // terminal `None` FName (the GVAS property-list end) that precedes the trailing
    // `GVAS`, and rebuild only the zero-pad between them. Guarded: we only touch it
    // when that gap is pure zero padding, so we never discard real bytes by
    // guessing — anything else falls through to the diagnostic error.
    if let Some(g) = rposition(plain, MAGIC) {
        const NONE_TERM: &[u8] = b"\x05\x00\x00\x00None\x00"; // int32 len 5 + "None\0"
        if let Some(n) = rposition(&plain[..g], NONE_TERM) {
            let body_end = n + NONE_TERM.len();
            if plain[body_end..g].iter().all(|&b| b == 0) {
                let footer = plain[g..].to_vec();
                repad_eoa(plain, body_end, &footer);
            }
        }
    }
    // Unrecognized trailer: leave `plain` as-is; `encrypt` surfaces the
    // misalignment and `write` attaches a copy-pasteable diagnostic.
}

/// Truncate `plain` to `body_len`, zero-pad so `body_len + pad + footer` is
/// 16-aligned, then append `footer` (the `GVAS…` trailer).
fn repad_eoa(plain: &mut Vec<u8>, body_len: usize, footer: &[u8]) {
    let pad = (16 - (body_len + footer.len()) % 16) % 16;
    plain.truncate(body_len);
    plain.resize(body_len + pad, 0);
    plain.extend_from_slice(footer);
}

/// Lowercase space-separated hex, for copy-pasteable diagnostics.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// Index of the LAST occurrence of `needle` in `hay` (there's no std slice rfind
/// for subslices). Used to find the trailing `GVAS` magic.
fn rposition(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Back up `path` (if it exists) to a timestamped file under a sibling
/// `backups/` folder — e.g. `backups/SaveData.sav.1720000000.bak`. Timestamped
/// so each save/apply keeps its own copy; nothing ever overwrites an old backup.
/// Returns the backup path (or `None` if there was nothing to back up).
pub fn backup(path: impl AsRef<Path>) -> Result<Option<PathBuf>, SaveError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("SaveData.sav");
    let mut bak = dir.join(format!("{name}.{stamp}.bak"));
    // If two saves land in the same second, disambiguate.
    let mut n = 1;
    while bak.exists() {
        bak = dir.join(format!("{name}.{stamp}-{n}.bak"));
        n += 1;
    }
    std::fs::copy(path, &bak)?;
    Ok(Some(bak))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::appearance::FieldValue;

    // Real-save round-trip. Skips cleanly when the local save/key aren't present
    // (e.g. CI / other machines) so it never breaks the public build.
    fn local() -> Option<(String, PathBuf)> {
        let key_path = dirs_home().join("eoa-backup/aes.key");
        let sav = dirs_home().join("eoa-backup/saves/SaveData.work.sav");
        if key_path.exists() && sav.exists() {
            Some((std::fs::read_to_string(key_path).ok()?, sav))
        } else {
            None
        }
    }
    fn dirs_home() -> PathBuf {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
    }

    // The real EoA trailer: alignment zero-pad, then `GVAS` + a fixed footer
    // block (version/flags). Tests must use this shape — an earlier fake
    // `[pad]GVAS` (footer-less) trailer let the rename-corruption bug through.
    const FOOTER: &[u8] = b"GVAS\x03\x00\x00\x00\x0a\x02\x00\x00\xf1";

    fn real_extra() -> Vec<u8> {
        [b"\0\0\0\0".as_slice(), FOOTER].concat()
    }

    #[test]
    fn realign_pads_to_16_and_keeps_footer() {
        let extra = real_extra();
        let mut plain = vec![0xAB; 13];
        plain.extend_from_slice(&extra);
        realign_eoa(&mut plain, &extra);
        assert_eq!(plain.len() % 16, 0, "must be 16-aligned");
        assert!(plain.ends_with(FOOTER), "footer (GVAS + version/flags) must survive at the end");
        assert_eq!(&plain[..13], &[0xAB; 13], "body preserved");
    }

    #[test]
    fn realign_is_noop_when_already_aligned() {
        let extra = real_extra(); // 4 pad + 13 footer = 17
        let mut plain = vec![0xAB; 15]; // 15 + 17 = 32; recomputed pad is also 4
        plain.extend_from_slice(&extra);
        let before = plain.clone();
        realign_eoa(&mut plain, &extra);
        assert_eq!(plain, before, "aligned save must reproduce byte-for-byte");
    }

    #[test]
    fn realign_recomputes_padding_for_any_length() {
        let extra = real_extra();
        for body in 1..48usize {
            let mut plain = vec![0xAB; body];
            plain.extend_from_slice(&extra);
            realign_eoa(&mut plain, &extra);
            assert_eq!(plain.len() % 16, 0, "body {body} not aligned");
            assert!(plain.ends_with(FOOTER), "body {body}: footer lost");
            assert_eq!(&plain[..body], &vec![0xAB; body][..], "body {body} corrupted");
        }
    }

    #[test]
    fn realign_footerless_zero_pad_trailer() {
        // Reported from the wild (0.1.11 rename on Windows): the save has NO
        // trailing GVAS footer — the trailer is just zero padding, and uesave
        // hands it over as an all-zeros 4-byte `extra`. A length-changing edit
        // then serialized to a non-multiple-of-16 and the editor refused.
        // realign must rebuild the pad (keeping the original zeros) instead.
        let extra = [0u8, 0, 0, 0];
        for body in 1..48usize {
            let mut plain = vec![0xAB; body];
            plain.extend_from_slice(b"\x05\x00\x00\x00None\0"); // GVAS terminator
            let body_len = plain.len();
            plain.extend_from_slice(&extra);
            realign_eoa(&mut plain, &extra);
            assert_eq!(plain.len() % 16, 0, "body {body}: not 16-aligned");
            assert_eq!(&plain[..body_len], &{
                let mut b = vec![0xAB; body];
                b.extend_from_slice(b"\x05\x00\x00\x00None\0");
                b
            }[..], "body {body}: body corrupted");
            assert!(
                plain[body_len..].iter().all(|&b| b == 0),
                "body {body}: trailer must stay all zeros"
            );
            assert!(
                plain.len() - body_len >= extra.len(),
                "body {body}: original zero trailer must not shrink"
            );
        }
    }

    #[test]
    fn realign_fallback_when_extra_lacks_gvas() {
        // The friend's case: uesave didn't hand us a GVAS-bearing `extra`, but the
        // serialized buffer still ends with `[body][zero pad][GVAS]`. realign must
        // find the trailer in the buffer and re-align without corrupting the body.
        // Body ends with a length-prefixed "None" terminator like a real save.
        let mut body = vec![0xAB; 20];
        body.extend_from_slice(&[0x05, 0x00, 0x00, 0x00]); // int32 len = 5
        body.extend_from_slice(b"None\0"); // the 5 string bytes
        let mut plain = body.clone();
        plain.extend_from_slice(&[0, 0, 0]); // some zero pad
        plain.extend_from_slice(b"GVAS"); // trailing magic
        realign_eoa(&mut plain, b""); // <-- empty extra forces the fallback
        assert_eq!(plain.len() % 16, 0, "fallback must 16-align");
        assert!(plain.ends_with(b"GVAS"), "GVAS must stay at the end");
        assert_eq!(&plain[..20], &[0xAB; 20], "body preserved");
        // The terminal "None" string must still read back its 5 declared bytes.
        let s = &plain[24..29];
        assert_eq!(s, b"None\0", "None terminator intact across the rebuilt pad");
    }

    #[test]
    fn realign_leaves_unknown_trailer_untouched() {
        let extra = b"NOMAGIC!"; // no GVAS anywhere
        let mut plain = vec![1u8; 13];
        plain.extend_from_slice(extra);
        let before = plain.clone();
        realign_eoa(&mut plain, extra);
        assert_eq!(plain, before);
    }

    #[test]
    fn unchanged_save_reencrypts_byte_identical() {
        let Some((key, sav)) = local() else { return };
        let file = SaveFile::load(&sav, key.trim()).expect("load");
        let raw = std::fs::read(&sav).unwrap();
        let mut plain = Vec::new();
        file.save.write(&mut plain).unwrap();
        let k = crypto::parse_key(key.trim()).unwrap();
        let reenc = crypto::encrypt(&k, &plain).unwrap();
        assert_eq!(reenc, raw, "unchanged save must re-encrypt byte-identical");
    }

    // Regression for "save length … not a multiple of 16": a length-CHANGING edit
    // (voice → the shorter bare `Player_M`, and rename) must write a 16-aligned
    // file through the REAL `write()` path and reload cleanly. This exercises
    // realign end-to-end on the real save, not just the synthetic-trailer unit
    // tests above (which use a fake footer). Length-changing edits are exactly the
    // ones that shift the body and were corrupting saves.
    // Regression for "save length … not a multiple of 16": EVERY length-changing
    // edit (the Identity group: HeroName / Gender / Voice — names + enums shift the
    // body) must write a 16-aligned file through the REAL `write()` path and reload
    // cleanly. Fixed-width fields (colours=floats, parts=ints, body=floats) can't
    // change length so they're not the suspect. Various string lengths incl.
    // shorter, longer, empty, accented, and multi-byte (emoji) names.
    #[test]
    fn length_changing_edits_write_aligned_and_reload() {
        let Some((key, sav)) = local() else { return };
        let out = std::env::temp_dir().join("aml-realign-regression.sav");
        let mut cases: Vec<(&str, FieldValue)> = vec![
            ("Voice", FieldValue::Name("Player_M".into())),        // shorter
            ("Voice", FieldValue::Name("Player_M_06".into())),     // longest
            ("HeroName", FieldValue::Str(String::new())),          // empty
            ("HeroName", FieldValue::Str("A".into())),             // 1 char
            ("HeroName", FieldValue::Str("Zoë".into())),           // accented (multi-byte)
            ("HeroName", FieldValue::Str("🗡️Kirito🗡️".into())),      // emoji
            ("HeroName", FieldValue::Str("a-fairly-long-hero-name-to-shift-body".into())),
        ];
        // Gender flips length via the enum string too.
        if let Some(g) = SaveFile::load(&sav, key.trim()).ok()
            .and_then(|f| f.appearance(0).ok())
            .and_then(|v| v.into_iter().find(|f| f.name == "Gender"))
        {
            if let FieldValue::Enum(s) = g.value {
                let flip = if s.contains("Female") { s.replace("Female", "Male") } else { s.replace("Male", "Female") };
                cases.push(("Gender", FieldValue::Enum(flip)));
            }
        }
        for (field, value) in cases {
            let mut file = SaveFile::load(&sav, key.trim()).expect("load");
            if file.appearance(0).unwrap().iter().all(|f| f.name != field) {
                continue;
            }
            file.set_appearance(0, field, value.clone()).expect("set field");
            file.write(&out).unwrap_or_else(|e| panic!("{field}={value:?}: write failed: {e}"));
            let enc = std::fs::read(&out).unwrap();
            assert_eq!(enc.len() % 16, 0, "{field}={value:?}: written save not 16-aligned");
            SaveFile::load(&out, key.trim()).unwrap_or_else(|e| panic!("{field}={value:?}: reload failed: {e}"));
        }
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn edit_reads_back() {
        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        assert!(file.character_count() >= 1);
        file.set_appearance(0, "Nose", FieldValue::Int(9)).expect("set nose");
        // Re-read from the in-memory tree.
        let fields = file.appearance(0).unwrap();
        let nose = fields.iter().find(|f| f.name == "Nose").unwrap();
        assert_eq!(nose.value, FieldValue::Int(9));
    }

    #[test]
    fn gender_enum_reads_and_writes() {
        // Regression: Gender is an EnumProperty ("ECharacterSex::Male"), not a Byte.
        // It must be readable and editable (it was silently dropped before).
        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let g = file.appearance(0).unwrap().into_iter().find(|f| f.name == "Gender");
        let orig = match g.map(|f| f.value) {
            Some(FieldValue::Enum(s)) => s,
            other => panic!("Gender not read as Enum: {other:?}"),
        };
        let flipped = if orig.ends_with("Male") && !orig.ends_with("Female") {
            orig.replace("Male", "Female")
        } else {
            orig.replace("Female", "Male")
        };
        file.set_appearance(0, "Gender", FieldValue::Enum(flipped.clone())).expect("set gender");
        let now = file.appearance(0).unwrap().into_iter().find(|f| f.name == "Gender").unwrap();
        assert_eq!(now.value, FieldValue::Enum(flipped));
    }

    #[test]
    fn part_id_validation_matches_creator_sets() {
        use crate::appearance::part_id_valid;
        // NPC hair ids must never pass — writing one into the save indexes off
        // the game's fixed hair-mesh array and breaks the character on load.
        assert!(!part_id_valid("HeadGearID", 800001));
        assert!(!part_id_valid("HeadGearID", 850505));
        assert!(!part_id_valid("HeadGearID", 1000)); // off-pattern junk
        assert!(part_id_valid("HeadGearID", 1001));
        assert!(part_id_valid("HeadGearID", 20001));
        assert!(!part_id_valid("Eyebrows", 13)); // creator skips this id
        assert!(part_id_valid("MoleID", 0)); // 0 = none is legal
        // Non-part fields have no id table and are never blocked.
        assert!(part_id_valid("MeshScale", 12345));
    }

    #[test]
    fn look_apply_skips_npc_hair_id() {
        // A shared/hand-edited look carrying an NPC hair must not reach the save.
        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let before = file.appearance(0).unwrap().into_iter()
            .find(|f| f.name == "HeadGearID").map(|f| f.value);
        let look = crate::preset::Look::from_json(
            r#"{"name":"evil","fields":[["HeadGearID",{"Int":800001}],["Nose",{"Int":3}]]}"#,
        ).unwrap();
        let applied = look.apply(&mut file, 0);
        assert_eq!(applied, 1, "only the valid Nose edit may apply");
        let after = file.appearance(0).unwrap().into_iter()
            .find(|f| f.name == "HeadGearID").map(|f| f.value);
        assert_eq!(before, after, "NPC hair id must not be written");
    }

    #[test]
    fn look_capture_json_apply_roundtrip() {
        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        // Capture -> JSON -> back.
        let look = crate::preset::Look::capture(&file, 0, "test").unwrap();
        let json = look.to_json().unwrap();
        let look2 = crate::preset::Look::from_json(&json).unwrap();
        assert_eq!(look.fields, look2.fields);
        // Mutate a field, then re-apply the captured look — value restored.
        let orig_nose = file.appearance(0).unwrap().into_iter().find(|f| f.name == "Nose").unwrap().value;
        file.set_appearance(0, "Nose", FieldValue::Int(1)).unwrap();
        let applied = look2.apply(&mut file, 0);
        assert!(applied > 0);
        let now = file.appearance(0).unwrap().into_iter().find(|f| f.name == "Nose").unwrap().value;
        assert_eq!(now, orig_nose, "applying the look must restore the captured Nose");
    }

    #[test]
    fn edit_surgical_then_restore() {
        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let original = file.appearance(0).unwrap();
        let orig_nose = original.iter().find(|f| f.name == "Nose").unwrap().value.clone();
        // change and restore -> tree back to identical serialization
        let raw_plain = { let mut v = Vec::new(); file.save.write(&mut v).unwrap(); v };
        file.set_appearance(0, "Nose", FieldValue::Int(9)).unwrap();
        file.set_appearance(0, "Nose", orig_nose).unwrap();
        let restored = { let mut v = Vec::new(); file.save.write(&mut v).unwrap(); v };
        assert_eq!(raw_plain, restored, "restore must reproduce the exact plaintext");
    }
}
