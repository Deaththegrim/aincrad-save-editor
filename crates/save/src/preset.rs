//! Appearance presets ("looks"): capture a character's editable appearance as a
//! standalone JSON file, separate from the encrypted save — so people can build a
//! library of looks and keep personal backups, then re-apply one to any character.

use crate::appearance::FieldValue;
use crate::{SaveError, SaveFile};
use serde::{Deserialize, Serialize};

/// A named snapshot of a character's appearance fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Look {
    pub name: String,
    /// Editor/version marker so future changes can migrate old looks.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// (field name, value) for every editable appearance leaf.
    pub fields: Vec<(String, FieldValue)>,
}

fn default_kind() -> String {
    "aml-look-v1".into()
}

impl Look {
    /// Capture the appearance of a character slot into a named look.
    pub fn capture(save: &SaveFile, slot: usize, name: impl Into<String>) -> Result<Self, SaveError> {
        let fields = save
            .appearance(slot)?
            .into_iter()
            .map(|f| (f.name, f.value))
            .collect();
        Ok(Look { name: name.into(), kind: default_kind(), fields })
    }

    /// Apply this look onto a character slot. Missing/incompatible fields are
    /// skipped (so a look captured on another build won't corrupt the save), and
    /// so is anything the game has no asset/meaning for: part ids the character
    /// creator doesn't offer (an NPC hair id like 800001 indexes off the end of
    /// the game's fixed mesh arrays and breaks the character), out-of-range body
    /// floats, unknown Voice/Gender values, and non-finite colour components.
    /// `HeroName` is never applied — a look is an appearance, not an identity,
    /// and applying a shared look must not silently rename the character.
    /// Returns how many fields were applied.
    pub fn apply(&self, save: &mut SaveFile, slot: usize) -> usize {
        let mut n = 0;
        for (name, value) in &self.fields {
            if name == "HeroName" {
                continue;
            }
            if !crate::appearance::identity_valid(name, value) {
                continue;
            }
            if let FieldValue::Int(v) = value {
                if !crate::appearance::part_id_valid(name, *v) {
                    continue;
                }
            }
            // Same treatment for body floats: an out-of-range morph weight
            // (hand-edited look, or captured on a broken save) extrapolates the
            // morph and warps the mesh — e.g. a Chest far below -1.0 pinches
            // the neck base. This also skips any MeshScale ≠ 1.0, so a look
            // can't re-introduce the global scale bug.
            if let FieldValue::Float(v) = value {
                if !crate::appearance::float_valid(name, *v) {
                    continue;
                }
            }
            if let FieldValue::Color(c) = value {
                if !crate::appearance::color_valid(c) {
                    continue;
                }
            }
            if save.set_appearance(slot, name, value.clone()).is_ok() {
                n += 1;
            }
        }
        save.note_edit(format!("applied look {:?} to slot {slot} ({n} of {} fields)", self.name, self.fields.len()));
        n
    }

    pub fn to_json(&self) -> Result<String, SaveError> {
        serde_json::to_string_pretty(self).map_err(|e| SaveError::Serialize(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, SaveError> {
        serde_json::from_str(s).map_err(|e| SaveError::Parse(e.to_string()))
    }
}
