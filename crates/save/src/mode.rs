//! Slot-level game-mode fields — `CharacterSaveData[slot].bDeathGameMode`.
//!
//! These live directly on the per-character slot struct, OUTSIDE `AvatarData`,
//! so the appearance walkers never see them and a shared look can never carry
//! one (a look is an appearance, not a life-or-death ruling).
//!
//! `bDeathGameMode` is the character-creation permadeath choice (verified in
//! the mapped save layout, echoes-of-aincrad-mods docs: `FCharacterSaveData`
//! field, set in-game via `ServerChangeDifficultyLevel(Level,
//! bNewDeathGameMode)`). Like every other save field, the game consumes it on
//! load with no validation — a plain bool flip is exactly what the game itself
//! writes.

use crate::appearance::{find, find_mut, slot_props, slot_props_mut};
use crate::SaveError;
use uesave::{Property, Save};

/// Read `bDeathGameMode` for a character slot.
pub fn death_game(save: &Save, slot: usize) -> Result<bool, SaveError> {
    match find(slot_props(save, slot)?, "bDeathGameMode") {
        Some(Property::Bool(b)) => Ok(*b),
        Some(_) => Err(SaveError::Parse("bDeathGameMode is not a Bool".into())),
        None => Err(SaveError::NoField("bDeathGameMode".into())),
    }
}

/// Write `bDeathGameMode` for a character slot. Type-checked like
/// [`crate::appearance::set`]: only an existing BoolProperty is written, never
/// created — a save without the field is left untouched.
pub(crate) fn set_death_game(save: &mut Save, slot: usize, value: bool) -> Result<(), SaveError> {
    match find_mut(slot_props_mut(save, slot)?, "bDeathGameMode") {
        Some(Property::Bool(b)) => {
            *b = value;
            Ok(())
        }
        Some(_) => Err(SaveError::Parse("bDeathGameMode is not a Bool".into())),
        None => Err(SaveError::NoField("bDeathGameMode".into())),
    }
}
