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
    /// skipped (so a look captured on another build won't corrupt the save).
    /// Returns how many fields were applied.
    pub fn apply(&self, save: &mut SaveFile, slot: usize) -> usize {
        let mut n = 0;
        for (name, value) in &self.fields {
            if save.set_appearance(slot, name, value.clone()).is_ok() {
                n += 1;
            }
        }
        n
    }

    pub fn to_json(&self) -> Result<String, SaveError> {
        serde_json::to_string_pretty(self).map_err(|e| SaveError::Serialize(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, SaveError> {
        serde_json::from_str(s).map_err(|e| SaveError::Parse(e.to_string()))
    }
}
