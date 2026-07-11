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
        (Property::Byte(slot), FieldValue::Enum(v)) => *slot = Byte::Label(v.clone()),
        (Property::Struct(StructValue::LinearColor(c)), FieldValue::Color(v)) => {
            *c = LinearColor { r: Float(v[0]), g: Float(v[1]), b: Float(v[2]), a: Float(v[3]) };
        }
        _ => return false, // type mismatch — don't corrupt
    }
    true
}
