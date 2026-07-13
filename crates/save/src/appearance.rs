//! Navigating the character appearance inside the GVAS property tree.
//!
//! Path: `root.CharacterSaveData[slot].AvatarData.{HeroName,Gender,Voice,
//! AvatarPartsData.*, AppearanceData.*}`. We expose the editable leaves as a flat
//! list of [`Field`]s grouped for the UI, and a [`set`] that writes one back.

use crate::SaveError;
use uesave::{Byte, Float, LinearColor, Properties, Property, Save, StructValue, ValueVec};

/// A single editable appearance value, tagged for the UI.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub group: Group,
    pub value: FieldValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Identity, // name, gender, voice
    Parts,    // head/headgear/mole/freckles IDs
    Face,     // eyebrows, eyeline, pupil, nose
    Body,     // mesh scale + sliders
    Color,    // LinearColor fields
    Toggle,   // bDefault*Color bools
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FieldValue {
    Int(i32),
    Float(f32),
    Bool(bool),
    /// HeroName (StrProperty).
    Str(String),
    /// Voice (NameProperty).
    Name(String),
    /// An enum property (Gender), e.g. "ECharacterSex::Male".
    Enum(String),
    /// LinearColor RGBA.
    Color([f32; 4]),
}

/// The `CharacterSaveData` array of character-slot structs.
pub(crate) fn slots(save: &Save) -> Option<&Vec<StructValue>> {
    match find(&save.root.properties, "CharacterSaveData")? {
        Property::Array(ValueVec::Struct(v)) => Some(v),
        _ => None,
    }
}

fn find<'a>(props: &'a Properties, name: &str) -> Option<&'a Property> {
    props.0.iter().find(|(k, _)| k.1 == name).map(|(_, v)| v)
}
fn find_mut<'a>(props: &'a mut Properties, name: &str) -> Option<&'a mut Property> {
    props.0.iter_mut().find(|(k, _)| k.1 == name).map(|(_, v)| v)
}
fn inner(p: &Property) -> Option<&Properties> {
    match p {
        Property::Struct(StructValue::Struct(pr)) => Some(pr),
        _ => None,
    }
}
fn inner_mut(p: &mut Property) -> Option<&mut Properties> {
    match p {
        Property::Struct(StructValue::Struct(pr)) => Some(pr),
        _ => None,
    }
}

/// Borrow the `AvatarData` properties of a slot.
fn avatar(save: &Save, slot: usize) -> Result<&Properties, SaveError> {
    let s = slots(save).ok_or_else(|| SaveError::Parse("no CharacterSaveData".into()))?;
    let StructValue::Struct(props) = s.get(slot).ok_or(SaveError::NoSlot(slot))? else {
        return Err(SaveError::NoSlot(slot));
    };
    find(props, "AvatarData").and_then(inner).ok_or_else(|| SaveError::NoField("AvatarData".into()))
}

fn avatar_mut(save: &mut Save, slot: usize) -> Result<&mut Properties, SaveError> {
    let s = match find_mut(&mut save.root.properties, "CharacterSaveData") {
        Some(Property::Array(ValueVec::Struct(v))) => v,
        _ => return Err(SaveError::Parse("no CharacterSaveData".into())),
    };
    let StructValue::Struct(props) = s.get_mut(slot).ok_or(SaveError::NoSlot(slot))? else {
        return Err(SaveError::NoSlot(slot));
    };
    find_mut(props, "AvatarData")
        .and_then(inner_mut)
        .ok_or_else(|| SaveError::NoField("AvatarData".into()))
}

fn value_of(p: &Property) -> Option<FieldValue> {
    Some(match p {
        Property::Int(v) => FieldValue::Int(*v),
        Property::Float(f) => FieldValue::Float(f.0),
        Property::Bool(b) => FieldValue::Bool(*b),
        Property::Str(s) => FieldValue::Str(s.clone()),
        Property::Name(s) => FieldValue::Name(s.clone()),
        Property::Enum(s) => FieldValue::Enum(s.clone()),
        Property::Byte(Byte::Label(s)) => FieldValue::Enum(s.clone()),
        Property::Byte(Byte::Byte(b)) => FieldValue::Enum(b.to_string()),
        Property::Struct(StructValue::LinearColor(c)) => {
            FieldValue::Color([c.r.0, c.g.0, c.b.0, c.a.0])
        }
        _ => return None,
    })
}

fn group_for(name: &str) -> Group {
    match name {
        "HeroName" | "Gender" | "Voice" => Group::Identity,
        "HeadGearID" | "HeadID" | "MoleID" | "FrecklesID" => Group::Parts,
        "Eyebrows" | "Eyeline" | "Pupil" | "Nose" => Group::Face,
        n if n.starts_with("bDefault") => Group::Toggle,
        n if n.starts_with("CustomColor")
            || n.contains("CustomColor")
            || n.ends_with("Color") =>
        {
            Group::Color
        }
        _ => Group::Body,
    }
}

/// One picker part: its save field, the editor's thumbnail folder, and every id
/// the game's character creator offers (verified against the game's
/// `AvatarCustomizeDataAsset` / `DT_*` thumbnail tables — the ids are NOT
/// contiguous). Single source of truth so the UI steppers and preset validation
/// can't drift apart. An id outside these sets (e.g. an NPC hair like 800001)
/// indexes off the end of the game's fixed mesh arrays and breaks the character.
pub struct PartIds {
    /// Save field name (what `Field::name` / `set` use), e.g. "HeadGearID".
    pub field: &'static str,
    /// Thumbnail-bundle folder name the editor UI uses, e.g. "HeadGear".
    pub folder: &'static str,
    pub ids: &'static [i32],
}

pub const PART_IDS: &[PartIds] = &[
    PartIds { field: "Nose", folder: "Nose", ids: &[1, 2, 3, 4, 5, 6, 7, 8] },
    PartIds {
        field: "Eyebrows",
        folder: "Eyebrow",
        ids: &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 18, 21, 22, 27, 28, 29],
    },
    PartIds {
        field: "Eyeline",
        folder: "Eyeline",
        ids: &[1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 17, 19, 20, 22, 23, 24, 27, 28, 29, 33, 34],
    },
    PartIds {
        field: "Pupil",
        folder: "Pupil",
        ids: &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    },
    PartIds {
        field: "HeadID",
        folder: "Jaw",
        ids: &[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 30, 31, 32, 33, 34, 35, 36, 37, 38,
        ],
    },
    PartIds {
        field: "HeadGearID",
        folder: "HeadGear",
        ids: &[
            1001, 2001, 3001, 4001, 5001, 6001, 7001, 8001, 9001, 10001, 11001, 12001, 13001,
            14001, 15001, 16001, 17001, 18001, 19001, 20001,
        ],
    },
    PartIds { field: "MoleID", folder: "Mole", ids: &[0, 1, 2, 3, 4, 5, 6, 7] },
    PartIds { field: "FrecklesID", folder: "Freckles", ids: &[0, 1, 2] },
];

/// Whether `value` is a game-valid id for the part field `field`. Fields with no
/// id table (colours, sliders, toggles) are always valid — only known part ids
/// are checked.
pub fn part_id_valid(field: &str, value: i32) -> bool {
    PART_IDS.iter().find(|p| p.field == field).is_none_or(|p| p.ids.contains(&value))
}

/// Body-shape float fields and their safe range: `(save field, min, max)`.
///
/// The morph weights run -1.0..=1.0 — the game's own char-creator span (from the
/// WBP_AvatarCustomize slider blueprints). Each maps to a `BS_BOD_*` morph
/// target on the costume meshes; outside the span the morph extrapolates and
/// warps the mesh (a chest far below -1.0 pinches the neck base, since the
/// `BS_BOD_Chest` deltas reach into it and there is no separate neck morph).
///
/// `MeshScale` is pinned to exactly 1.0: the character creator never exposes
/// it, and a drifted value resizes every character and mob in the game (the
/// "scale bug" — see the editor's fix_scale). Single source of truth for the
/// UI sliders AND preset validation, like [`PART_IDS`].
pub const FLOAT_RANGES: &[(&str, f32, f32)] = &[
    ("Chest", -1.0, 1.0),
    ("Arms", -1.0, 1.0),
    ("ForeArms", -1.0, 1.0),
    ("Hands", -1.0, 1.0),
    ("Belly", -1.0, 1.0),
    ("Butts", -1.0, 1.0),
    ("Hips", -1.0, 1.0),
    ("Thighs", -1.0, 1.0),
    ("Legs", -1.0, 1.0),
    ("Feet", -1.0, 1.0),
    ("MeshScale", 1.0, 1.0),
];

/// Whether `value` is inside the safe range for the float field `field`.
/// Fields with no range entry are always valid; NaN never is.
pub fn float_valid(field: &str, value: f32) -> bool {
    FLOAT_RANGES
        .iter()
        .find(|(f, _, _)| *f == field)
        .is_none_or(|(_, lo, hi)| (*lo..=*hi).contains(&value))
}

/// Read every editable appearance field for a character slot.
pub fn read(save: &Save, slot: usize) -> Result<Vec<Field>, SaveError> {
    let av = avatar(save, slot)?;
    let mut out = Vec::new();

    // Identity leaves live directly on AvatarData.
    for key in ["HeroName", "Gender", "Voice"] {
        if let Some(v) = find(av, key).and_then(value_of) {
            out.push(Field { name: key.into(), group: Group::Identity, value: v });
        }
    }
    // AvatarPartsData + AppearanceData sub-structs.
    for sub in ["AvatarPartsData", "AppearanceData"] {
        if let Some(props) = find(av, sub).and_then(inner) {
            for (k, p) in props.0.iter() {
                if let Some(v) = value_of(p) {
                    out.push(Field { name: k.1.clone(), group: group_for(&k.1), value: v });
                }
            }
        }
    }
    Ok(out)
}

/// Set one appearance field by name (searched across AvatarData + its two
/// sub-structs). Type must match the existing field.
pub fn set(save: &mut Save, slot: usize, name: &str, value: FieldValue) -> Result<(), SaveError> {
    let av = avatar_mut(save, slot)?;

    // Try the AvatarData leaves, then each sub-struct.
    if apply(av, name, &value) {
        return Ok(());
    }
    for sub in ["AvatarPartsData", "AppearanceData"] {
        if let Some(props) = find_mut(av, sub).and_then(inner_mut) {
            if apply(props, name, &value) {
                return Ok(());
            }
        }
    }
    Err(SaveError::NoField(name.into()))
}

/// Write `value` into `props[name]` if present and type-compatible. Returns true
/// if it was found and applied.
fn apply(props: &mut Properties, name: &str, value: &FieldValue) -> bool {
    let Some(p) = find_mut(props, name) else {
        return false;
    };
    match (p, value) {
        (Property::Int(slot), FieldValue::Int(v)) => *slot = *v,
        (Property::Float(slot), FieldValue::Float(v)) => *slot = Float(*v),
        (Property::Bool(slot), FieldValue::Bool(v)) => *slot = *v,
        (Property::Str(slot), FieldValue::Str(v)) => *slot = v.clone(),
        (Property::Name(slot), FieldValue::Name(v)) => *slot = v.clone(),
        (Property::Enum(slot), FieldValue::Enum(v)) => *slot = v.clone(),
        // Preserve the byte's on-disk representation: a numeric ByteProperty must
        // stay numeric (writing a label into it changes the serialized form and
        // can corrupt the save). Refuse if the enum text isn't a number.
        (Property::Byte(slot), FieldValue::Enum(v)) => match slot {
            Byte::Byte(_) => match v.parse::<u8>() {
                Ok(n) => *slot = Byte::Byte(n),
                Err(_) => return false,
            },
            Byte::Label(_) => *slot = Byte::Label(v.clone()),
        },
        (Property::Struct(StructValue::LinearColor(c)), FieldValue::Color(v)) => {
            *c = LinearColor { r: Float(v[0]), g: Float(v[1]), b: Float(v[2]), a: Float(v[3]) };
        }
        _ => return false, // type mismatch — don't corrupt
    }
    true
}
