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
