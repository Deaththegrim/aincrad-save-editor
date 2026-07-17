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
pub mod mode;
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
         loaded length: {loaded_len} (decrypted; was 16-aligned on disk)\n\
         uesave extra ({extra_len} bytes): {extra_hex}\n\
         last 32 bytes: {tail_hex}\n\
         edits since load: {edits}\n\
         ---------------------------"
    )]
    Unaligned {
        len: usize,
        loaded_len: usize,
        extra_len: usize,
        extra_hex: String,
        tail_hex: String,
        edits: String,
    },
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
    /// Decrypted length at load time (16-aligned, since the file decrypted).
    /// Diagnostic-only: lets a trailer-refusal report show the length delta.
    loaded_len: usize,
    /// Coalesced edit journal for diagnostics: `(key, first old, latest new)`
    /// per edited field, in first-edit order. Marker lines (e.g. a preset
    /// apply) have empty old/new. Field VALUES are recorded except HeroName,
    /// which is reduced to length/encoding — the report gets pasted publicly.
    journal: Vec<(String, String, String)>,
}

impl SaveFile {
    /// Decrypt + parse the save at `path` using a hex AES-256 key.
    pub fn load(path: impl AsRef<Path>, hex_key: &str) -> Result<Self, SaveError> {
        let key = crypto::parse_key(hex_key)?;
        let raw = std::fs::read(&path)?;
        let plain = crypto::decrypt(&key, &raw)?;
        let save = Save::read(&mut std::io::Cursor::new(&plain))
            .map_err(|e| SaveError::Parse(e.to_string()))?;
        Ok(Self {
            key,
            save,
            source: path.as_ref().to_path_buf(),
            loaded_len: plain.len(),
            journal: Vec::new(),
        })
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
                loaded_len: self.loaded_len,
                extra_len: self.save.extra.len(),
                extra_hex: hex(&self.save.extra),
                tail_hex: hex(tail),
                edits: self.edits_summary(),
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

    /// Set one appearance field on a slot. Records the change in the edit
    /// journal (coalesced per field) so a later save-failure diagnostic can
    /// show what was edited.
    pub fn set_appearance(
        &mut self,
        slot: usize,
        name: &str,
        value: appearance::FieldValue,
    ) -> Result<(), SaveError> {
        let old = appearance::read(&self.save, slot)
            .ok()
            .and_then(|fs| fs.into_iter().find(|f| f.name == name))
            .map(|f| fmt_field(name, &f.value))
            .unwrap_or_else(|| "?".into());
        appearance::set(&mut self.save, slot, name, value.clone())?;
        let new = fmt_field(name, &value);
        self.journal_set(format!("slot{slot}.{name}"), old, new);
        Ok(())
    }

    /// Read the character-creation permadeath flag (`bDeathGameMode`) for a slot.
    pub fn death_game_mode(&self, slot: usize) -> Result<bool, SaveError> {
        mode::death_game(&self.save, slot)
    }

    /// Set the permadeath flag for a slot. Journaled like appearance edits.
    pub fn set_death_game_mode(&mut self, slot: usize, value: bool) -> Result<(), SaveError> {
        let old = mode::death_game(&self.save, slot)
            .map(|b| b.to_string())
            .unwrap_or_else(|_| "?".into());
        mode::set_death_game(&mut self.save, slot, value)?;
        self.journal_set(format!("slot{slot}.bDeathGameMode"), old, value.to_string());
        Ok(())
    }

    /// Record a field change in the edit journal, coalescing repeated edits to
    /// the same key and dropping an entry that returns to its original value.
    fn journal_set(&mut self, key: String, old: String, new: String) {
        if let Some(i) = self.journal.iter().position(|(k, _, _)| *k == key) {
            if self.journal[i].1 == new {
                // Edited back to the original value — not a change anymore.
                self.journal.remove(i);
            } else {
                self.journal[i].2 = new;
            }
        } else if old != new {
            self.journal.push((key, old, new));
        }
    }

    /// Append a free-form marker to the edit journal (e.g. "applied look X"),
    /// so a save-failure diagnostic shows the operation, not just its fields.
    pub fn note_edit(&mut self, marker: impl Into<String>) {
        self.journal.push((marker.into(), String::new(), String::new()));
    }

    /// The edit journal as one diagnostic string (see [`SaveError::Unaligned`]).
    fn edits_summary(&self) -> String {
        edits_summary(&self.journal)
    }
}

/// Render the edit journal for the copy-pasteable diagnostic: one indented
/// line per coalesced field change, `* `-prefixed lines for operation markers.
fn edits_summary(journal: &[(String, String, String)]) -> String {
    if journal.is_empty() {
        return "none recorded (length change came from re-serialization alone)".into();
    }
    journal
        .iter()
        .map(|(key, old, new)| {
            if old.is_empty() && new.is_empty() {
                format!("\n  * {key}")
            } else {
                format!("\n  {key}: {old} -> {new}")
            }
        })
        .collect()
}

/// Render a field value for the edit journal. HeroName is reduced to its
/// length + encoding class (the diagnostic gets pasted on public forums, and
/// for alignment bugs the serialized size is what matters — UE FStrings write
/// ASCII as 1 byte/char and anything else as UTF-16).
fn fmt_field(name: &str, v: &appearance::FieldValue) -> String {
    use appearance::FieldValue as FV;
    match v {
        FV::Str(s) if name == "HeroName" => {
            let enc = if s.is_ascii() { "ascii" } else { "non-ascii/utf16" };
            format!("str({} chars, {enc})", s.chars().count())
        }
        FV::Str(s) => format!("{s:?}"),
        FV::Name(s) | FV::Enum(s) => s.clone(),
        FV::Int(n) => n.to_string(),
        FV::Float(f) => format!("{f}"),
        FV::Bool(b) => b.to_string(),
        FV::Color(c) => format!("rgba({}, {}, {}, {})", c[0], c[1], c[2], c[3]),
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
    // Clipped-magic variant (from the wild, 0.1.11 on Windows): the trailer is
    // `[zero pad][a 1-3 byte PREFIX of the GVAS magic]` — the game's writer cut
    // the footer at the 16-aligned end of file (diagnostic showed a 5-byte
    // `extra` of 4 zeros + a lone b'G'). Keep the original extra verbatim
    // (pad-or-field reasoning as above; never shrink the original trailer) and
    // rebuild the alignment zeros in front of it. A full `GVAS` is handled by
    // the primary branch, so the prefix here is strictly shorter than 4 bytes.
    if !extra.is_empty() && plain.ends_with(extra) {
        let nz = extra.iter().position(|&b| b != 0).unwrap_or(extra.len());
        let tail = &extra[nz..];
        if !tail.is_empty() && tail.len() < MAGIC.len() && MAGIC.starts_with(tail) {
            let body_len = plain.len() - extra.len();
            repad_eoa(plain, body_len, extra);
            return;
        }
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
    fn realign_clipped_magic_trailer() {
        // Reported from the wild (0.1.11 on Windows): `extra` is 4 zero-pad
        // bytes + a lone b'G' — the game clipped the GVAS magic at the aligned
        // end of file. realign must keep the original trailer verbatim and
        // rebuild alignment zeros in front, never corrupting the body.
        for clip in 1..4usize {
            let extra: Vec<u8> = [&[0u8, 0, 0, 0][..], &b"GVAS"[..clip]].concat();
            for body in 1..48usize {
                let mut expected_body = vec![0xAB; body];
                expected_body.extend_from_slice(b"\x05\x00\x00\x00None\0");
                let mut plain = expected_body.clone();
                plain.extend_from_slice(&extra);
                realign_eoa(&mut plain, &extra);
                assert_eq!(plain.len() % 16, 0, "clip {clip} body {body}: not 16-aligned");
                assert!(
                    plain.ends_with(&extra),
                    "clip {clip} body {body}: original trailer must survive at the end"
                );
                assert_eq!(
                    &plain[..expected_body.len()],
                    &expected_body[..],
                    "clip {clip} body {body}: body corrupted"
                );
                assert!(
                    plain[expected_body.len()..plain.len() - extra.len()].iter().all(|&b| b == 0),
                    "clip {clip} body {body}: rebuilt pad must be zeros"
                );
            }
        }
        // The exact lengths from the wild diagnostic: serialized 450030 with a
        // 5-byte extra → 450032 after realign (pad of 2 rebuilt in front).
        let extra = [0u8, 0, 0, 0, b'G'];
        let mut plain = vec![0xCD; 450_030 - extra.len()];
        plain.extend_from_slice(&extra);
        realign_eoa(&mut plain, &extra);
        assert_eq!(plain.len(), 450_032);
        assert!(plain.ends_with(&extra));
        // Second wild diagnostic (0.1.11, Windows): serialized 138085 with a
        // 6-byte extra of 4 zeros + b"GV" → 138096 after realign (pad of 11).
        let extra = [0u8, 0, 0, 0, b'G', b'V'];
        let mut plain = vec![0xCD; 138_085 - extra.len()];
        plain.extend_from_slice(&extra);
        realign_eoa(&mut plain, &extra);
        assert_eq!(plain.len(), 138_096);
        assert!(plain.ends_with(&extra));
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
    fn journal_fmt_redacts_hero_name_but_not_others() {
        use appearance::FieldValue as FV;
        // HeroName gets pasted on public forums — only size/encoding survive.
        assert_eq!(fmt_field("HeroName", &FV::Str("Kirito".into())), "str(6 chars, ascii)");
        assert_eq!(fmt_field("HeroName", &FV::Str("キリト".into())), "str(3 chars, non-ascii/utf16)");
        // Everything else is game data, not personal — keep the real values.
        assert_eq!(fmt_field("Voice", &FV::Name("Player_M_02".into())), "Player_M_02");
        assert_eq!(fmt_field("Gender", &FV::Enum("ECharacterSex::Male".into())), "ECharacterSex::Male");
        assert_eq!(fmt_field("HeadGearID", &FV::Int(3001)), "3001");
        assert_eq!(fmt_field("Chest", &FV::Float(-0.5)), "-0.5");
    }

    #[test]
    fn journal_summary_formats_changes_and_markers() {
        assert_eq!(
            edits_summary(&[]),
            "none recorded (length change came from re-serialization alone)"
        );
        let j = vec![
            ("slot0.Voice".into(), "Player_F".into(), "Player_M_02".into()),
            ("applied look \"x\" to slot 0 (3 of 40 fields)".into(), String::new(), String::new()),
        ];
        let s = edits_summary(&j);
        assert_eq!(
            s,
            "\n  slot0.Voice: Player_F -> Player_M_02\n  * applied look \"x\" to slot 0 (3 of 40 fields)"
        );
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
    fn look_apply_skips_out_of_range_floats() {
        // Reported from the wild: a chest morph pushed far past the creator's
        // -1..1 span extrapolates the BS_BOD_Chest morph and pinches the neck
        // ("pipe-cleaner neck"). The UI sliders clamp, so the only editor path
        // to such a value is a shared/hand-edited look — apply must skip it.
        // MeshScale ≠ 1.0 must also be skipped (the global scale bug).
        use crate::appearance::float_valid;
        assert!(float_valid("Chest", -1.0));
        assert!(float_valid("Chest", 1.0));
        assert!(!float_valid("Chest", -3.0));
        assert!(!float_valid("Chest", f32::NAN));
        assert!(float_valid("MeshScale", 1.0));
        assert!(!float_valid("MeshScale", 0.6));
        assert!(float_valid("SomeUnknownFloat", 42.0)); // no table → not blocked

        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let before_chest = file.appearance(0).unwrap().into_iter()
            .find(|f| f.name == "Chest").map(|f| f.value);
        let look = crate::preset::Look::from_json(
            r#"{"name":"warped","fields":[["Chest",{"Float":-3.0}],["MeshScale",{"Float":0.5}],["Nose",{"Int":3}]]}"#,
        ).unwrap();
        let applied = look.apply(&mut file, 0);
        assert_eq!(applied, 1, "only the valid Nose edit may apply");
        let after_chest = file.appearance(0).unwrap().into_iter()
            .find(|f| f.name == "Chest").map(|f| f.value);
        assert_eq!(before_chest, after_chest, "out-of-range Chest must not be written");
    }

    #[test]
    fn look_apply_skips_identity_junk_and_never_renames() {
        // Names/Enums/Colors had NO validation on preset apply: a hand-edited
        // look could write a Voice the game has no audio for, a Gender outside
        // the game's 2-value ECharacterSex enum, a NaN colour component, or —
        // worst — silently RENAME the character (a look is an appearance, not
        // an identity).
        use crate::appearance::{color_valid, identity_valid};
        assert!(identity_valid("Voice", &FieldValue::Name("Player_F_06".into())));
        assert!(!identity_valid("Voice", &FieldValue::Name("Player_M_99".into())));
        assert!(!identity_valid("Voice", &FieldValue::Name("Player_M_01".into()))); // no _01
        assert!(identity_valid("Gender", &FieldValue::Enum("ECharacterSex::Female".into())));
        assert!(!identity_valid("Gender", &FieldValue::Enum("ECharacterSex::Banana".into())));
        assert!(color_valid(&[0.5, 0.5, 0.5, 1.0]));
        assert!(!color_valid(&[f32::NAN, 0.5, 0.5, 1.0]));
        assert!(!color_valid(&[0.5, f32::INFINITY, 0.5, 1.0]));

        let Some((key, sav)) = local() else { return };
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let field = |f: &SaveFile, name: &str| {
            f.appearance(0).unwrap().into_iter().find(|x| x.name == name).map(|x| x.value)
        };
        let before_name = field(&file, "HeroName");
        let before_voice = field(&file, "Voice");
        let before_gender = field(&file, "Gender");
        let before_skin = field(&file, "CustomColorSkin");
        // Built directly (not via JSON): serde_json rejects non-finite literals,
        // but a look written by other tooling can still carry them.
        let look = crate::preset::Look {
            name: "junk".into(),
            kind: "aml-look-v1".into(),
            fields: vec![
                ("HeroName".into(), FieldValue::Str("Imposter".into())),
                ("Voice".into(), FieldValue::Name("Player_M_99".into())),
                ("Gender".into(), FieldValue::Enum("ECharacterSex::Banana".into())),
                ("CustomColorSkin".into(), FieldValue::Color([f32::INFINITY, 0.5, 0.5, 1.0])),
                ("Nose".into(), FieldValue::Int(3)),
            ],
        };
        let applied = look.apply(&mut file, 0);
        assert_eq!(applied, 1, "only the valid Nose edit may apply");
        assert_eq!(before_name, field(&file, "HeroName"), "a look must never rename");
        assert_eq!(before_voice, field(&file, "Voice"), "unknown voice must not be written");
        assert_eq!(before_gender, field(&file, "Gender"), "bogus gender must not be written");
        assert_eq!(before_skin, field(&file, "CustomColorSkin"), "non-finite colour must not be written");
    }

    #[test]
    fn every_save_field_is_covered_by_a_validation_table() {
        // Schema-drift guard: if a game patch adds a new part id or body slider
        // to the save, it would flow through preset apply UNVALIDATED (unknown
        // fields default to "always valid"). Fail here first, so the new field
        // gets a verified entry in PART_IDS / FLOAT_RANGES before shipping.
        let Some((key, sav)) = local() else { return };
        let file = SaveFile::load(&sav, key.trim()).expect("load");
        for f in file.appearance(0).unwrap() {
            match f.value {
                FieldValue::Int(_) => assert!(
                    appearance::PART_IDS.iter().any(|p| p.field == f.name),
                    "Int field {} has no PART_IDS entry — verify its creator set and add it",
                    f.name
                ),
                FieldValue::Float(_) => assert!(
                    appearance::FLOAT_RANGES.iter().any(|(n, _, _)| *n == f.name),
                    "Float field {} has no FLOAT_RANGES entry — verify its creator span and add it",
                    f.name
                ),
                FieldValue::Name(_) | FieldValue::Enum(_) => assert!(
                    matches!(f.name.as_str(), "Voice" | "Gender"),
                    "Name/Enum field {} has no identity validation — add it to identity_valid",
                    f.name
                ),
                // Strs (HeroName), Bools and Colors have blanket handling.
                _ => {}
            }
        }
    }

    #[test]
    fn death_game_flag_reads_flips_and_roundtrips() {
        let Some((key, sav)) = local() else { return };
        let out = std::env::temp_dir().join("aml-deathgame-roundtrip.sav");
        let mut file = SaveFile::load(&sav, key.trim()).expect("load");
        let orig = file.death_game_mode(0).expect("real save must expose bDeathGameMode");
        // Flip, journal, write through the REAL write() path, reload, verify.
        file.set_death_game_mode(0, !orig).expect("set flag");
        assert_eq!(file.death_game_mode(0).unwrap(), !orig, "flip must read back");
        assert!(
            file.edits_summary().contains("slot0.bDeathGameMode"),
            "flag edits must be journaled for the trailer diagnostic"
        );
        file.write(&out).expect("write flipped save");
        let reloaded = SaveFile::load(&out, key.trim()).expect("reload");
        assert_eq!(reloaded.death_game_mode(0).unwrap(), !orig, "flip must survive a save/load");
        // Flip back = no longer an edit (journal coalescing) and byte-identical tree.
        file.set_death_game_mode(0, orig).unwrap();
        assert!(
            !file.edits_summary().contains("bDeathGameMode"),
            "restoring the original value must drop the journal entry"
        );
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn look_never_captures_death_game_mode() {
        // The flag lives on the slot struct, outside AvatarData — a captured
        // look must never include it (a shared look changing someone's
        // permadeath ruling would be a nasty surprise). Pin the boundary.
        let Some((key, sav)) = local() else { return };
        let file = SaveFile::load(&sav, key.trim()).expect("load");
        let look = crate::preset::Look::capture(&file, 0, "boundary").unwrap();
        assert!(
            look.fields.iter().all(|(name, _)| name != "bDeathGameMode"),
            "a look must never carry bDeathGameMode"
        );
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
