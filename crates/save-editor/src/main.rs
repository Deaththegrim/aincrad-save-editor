//! Aincrad Save Editor — a friendly visual editor for Echoes of Aincrad character
//! appearance. Decrypts the save (via aml-save), shows the real in-game part
//! thumbnails so players pick a face/hair/eyes by sight rather than by number,
//! and writes changes back safely (work copy first, live save only on confirm,
//! always with a timestamped backup).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The no-keyscan build gates out the key-recovery UI + its helpers; that leaves
// the recovery code paths + their i18n labels legitimately unused there.
#![cfg_attr(not(feature = "keyscan"), allow(dead_code))]

mod i18n;
mod locate;
mod npchair;
mod thumbs;
mod update;
mod voice_preview;

/// Key-scanner shim. The real scanner (reads the running game's memory to recover
/// the AES key) is compiled in only for the `keyscan` build. The no-keyscan build
/// (`--no-default-features`, shipped to Nexus / the split repo) gets stubs, so the
/// binary contains no `OpenProcess`/`ReadProcessMemory` imports that trip AV.
#[cfg(feature = "keyscan")]
mod ks {
    pub use aml_keyscan::{find_game_exe, find_game_pid, recover_key};
}
#[cfg(not(feature = "keyscan"))]
mod ks {
    #![allow(dead_code)]
    use std::path::{Path, PathBuf};
    pub fn find_game_pid() -> Option<u32> {
        None
    }
    pub fn find_game_exe() -> Option<PathBuf> {
        None
    }
    pub fn recover_key(_pak: &Path) -> Result<String, String> {
        Err("This build has no key scanner — enter your AES key manually.".into())
    }
}

use aml_save::appearance::{Field, FieldValue, Group};
use aml_save::preset::Look;
use aml_save::SaveFile;
use aml_ui::theme;
use egui::RichText;
use i18n::{Lang, S};
use std::path::{Path, PathBuf};

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_title("Aincrad Save Editor")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("bundled window icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Aincrad Save Editor",
        opts,
        Box::new(|cc| {
            // Pin dark mode BEFORE styling: eframe defaults to following the OS
            // theme, and set_visuals only writes the ACTIVE theme's style — on a
            // light-mode Windows the app flipped to stock egui light and our
            // palette never applied.
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            cc.egui_ctx.set_visuals(theme::visuals());
            install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

/// Add a CJK font as a fallback so Japanese / Chinese / Korean render (the default
/// egui font only covers Latin + Cyrillic). Loaded at runtime from the bundled
/// font (aml-data/fonts) or a common system font; if none is found, the CJK
/// languages fall back to boxes but the Latin/Cyrillic languages are unaffected.
fn install_cjk_font(ctx: &egui::Context) {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("aml-data/fonts"))) {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "ttc" || x == "otf" || x == "ttf") {
                    candidates.push(p);
                }
            }
        }
    }
    for p in [
        // Linux (Noto CJK)
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        // Windows
        "C:/Windows/Fonts/YuGothM.ttc",
        "C:/Windows/Fonts/msgothic.ttc",
        "C:/Windows/Fonts/malgun.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simsun.ttc",
    ] {
        candidates.push(PathBuf::from(p));
    }
    let Some(bytes) = candidates.into_iter().find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("cjk".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// A thumbnail picker: save field, thumbnail folder, and whether "None"=0 is valid.
/// The visible label is resolved per-language by [`picker_label`].
struct Picker {
    field: &'static str,
    folder: &'static str,
    optional: bool,
}
const fn pk(field: &'static str, folder: &'static str, optional: bool) -> Picker {
    Picker { field, folder, optional }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Identity,
    Face,
    Hair,
    Body,
    Looks,
    Backups,
}
const CATEGORY_ORDER: &[Category] = &[
    Category::Identity,
    Category::Face,
    Category::Hair,
    Category::Body,
    Category::Looks,
    Category::Backups,
];

fn cat_label(t: &S, cat: Category) -> &'static str {
    match cat {
        Category::Identity => t.cat_identity,
        Category::Face => t.cat_face,
        Category::Hair => t.cat_hair,
        Category::Body => t.cat_body,
        Category::Looks => t.cat_looks,
        Category::Backups => t.cat_backups,
    }
}

/// One row on the Backups page.
#[derive(Clone)]
struct BackupEntry {
    path: PathBuf,
    /// Unix seconds, from the `.{ts}.bak` filename (mtime fallback).
    ts: i64,
    /// Backup of the live save (true) or of the working copy (false).
    live: bool,
    size: u64,
}

/// The `<name>.{unix-seconds}.bak` timestamp, if the name has one.
fn backup_ts_from_name(name: &str) -> Option<i64> {
    let stem = name.strip_suffix(".bak")?;
    stem.rsplit('.').next()?.parse().ok()
}

/// All `.bak` files in one backups folder, tagged with their source.
fn scan_backup_dir(dir: &Path, live: bool) -> Vec<BackupEntry> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let path = e.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
            if !name.ends_with(".bak") {
                continue;
            }
            let meta = e.metadata().ok();
            let ts = backup_ts_from_name(name)
                .or_else(|| {
                    meta.as_ref()?
                        .modified()
                        .ok()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_secs() as i64)
                })
                .unwrap_or(0);
            v.push(BackupEntry { path, ts, live, size: meta.map(|m| m.len()).unwrap_or(0) });
        }
    }
    v
}

/// The translated label for a thumbnail picker (matched by its save field).
fn picker_label(t: &S, field: &str) -> &'static str {
    match field {
        "Nose" => t.nose,
        "Eyebrows" => t.eyebrows,
        "Eyeline" => t.eye_shape,
        "Pupil" => t.eyes_iris,
        "HeadID" => t.head_shape,
        "HeadGearID" => t.hair,
        "MoleID" => t.mole,
        "FrecklesID" => t.freckles,
        _ => "",
    }
}

/// Which category a colour / colour-toggle field belongs beside.
fn colour_category(name: &str) -> Category {
    if name.contains("Hair") || name.contains("Beard") {
        Category::Hair
    } else if name.contains("Skin")
        || name.contains("Upper")
        || name.contains("Gloves")
        || name.contains("Lower")
    {
        Category::Body
    } else {
        Category::Face // Face, Pupil, Eye, Eyeline, Eyebrow, Lip, Accessory
    }
}

// Face-feature pickers (direct 1-based IDs).
const FACE: &[Picker] = &[
    pk("Nose", "Nose", false),
    pk("Eyebrows", "Eyebrow", false),
    pk("Eyeline", "Eyeline", false),
    pk("Pupil", "Pupil", false),
    pk("HeadID", "Jaw", false),
];
// Hair + optional extras (IDs from the DataTable, e.g. hair 3001).
const HAIR: &[Picker] = &[
    pk("HeadGearID", "HeadGear", false),
    pk("MoleID", "Mole", true),
    pk("FrecklesID", "Freckles", true),
];

struct App {
    key: Option<String>,
    live_path: Option<PathBuf>,
    work_path: PathBuf,
    save: Option<SaveFile>,
    fields: Vec<Field>,
    thumbs: thumbs::ThumbCache,
    slot: usize,
    category: Category,
    status: String,
    dirty: bool,
    confirm_live: bool,
    new_look_name: String,
    looks: Vec<PathBuf>,
    thumb_scale: f32,
    key_input: String,
    /// In-flight background key recovery (channel delivers the result).
    recovery: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// The loaded save has a non-1.0 body scale (MeshScale) on some slot, which
    /// resizes every character/mob in-game — offer a one-click fix.
    scale_bug: bool,
    lang: Lang,
    /// Path to the hairswap mod's config file, if the mod is installed (resolved
    /// once at startup). `None` hides the NPC-hair section.
    hairswap_cfg: Option<PathBuf>,
    /// Currently-configured NPC hair id (mirrors the hairswap config file).
    npc_hair: Option<u32>,
    /// A save failure to surface in a copy-pasteable modal (full diagnostic text).
    save_error: Option<String>,
    /// In-flight background "newer release?" check (yields at most once).
    update_rx: Option<std::sync::mpsc::Receiver<update::Update>>,
    /// A newer published release, once the check confirms one — shown as a
    /// subtle top-bar link.
    update: Option<update::Update>,
    /// Plays the creator's voice sample lines from the bundled clips; the whole
    /// preview UI hides itself when the clips payload is absent.
    preview: voice_preview::VoicePreview,
    /// Backups page rows, rescanned when the page is opened or a backup is made.
    backups: Vec<BackupEntry>,
    /// Colour fields whose game-palette swatch strip is expanded (UI-only).
    swatches_open: std::collections::HashSet<String>,
}

impl App {
    fn new() -> Self {
        let cfg = aml_host::config::AppConfig::load();
        let key = cfg.aes_key;
        let lang = cfg.lang.as_deref().map(Lang::from_code).unwrap_or(Lang::En);
        let live_path = locate::find_save();
        let hairswap_cfg = npchair::config_path();
        let npc_hair = hairswap_cfg.as_deref().and_then(npchair::read);
        let mut app = App {
            key,
            live_path,
            work_path: locate::work_copy_path(),
            save: None,
            fields: Vec::new(),
            thumbs: thumbs::ThumbCache::new(locate::thumbs_dir()),
            slot: 0,
            category: Category::Identity,
            status: String::new(),
            dirty: false,
            confirm_live: false,
            new_look_name: String::new(),
            looks: Vec::new(),
            thumb_scale: 1.0,
            key_input: String::new(),
            recovery: None,
            scale_bug: false,
            lang,
            hairswap_cfg,
            npc_hair,
            save_error: None,
            update_rx: Some(update::spawn_check()),
            update: None,
            preview: voice_preview::VoicePreview::new(
                locate::voices_dir(),
                // Audio defaults to the UI language when it's one the game dubs.
                if lang == Lang::Ja {
                    voice_preview::AudioLang::Jp
                } else {
                    voice_preview::AudioLang::En
                },
            ),
            backups: Vec::new(),
            swatches_open: std::collections::HashSet::new(),
        };
        app.scan_looks();
        app.scan_backups();
        if app.key.is_none() {
            app.status = "Enter your Echoes of Aincrad pak AES key to begin.".into();
        } else if app.live_path.is_some() {
            app.load();
        } else {
            app.status = "Save not found automatically — use “Open save…”.".into();
        }
        app
    }

    /// Validate + store the pasted AES key (portable config next to the exe), then
    /// try to load the save.
    fn set_key(&mut self) {
        let k = self.key_input.trim().to_string();
        if aml_save::crypto::parse_key(&k).is_err() {
            self.note("That doesn't look like a 32-byte (64 hex-char) AES key.");
            return;
        }
        let mut cfg = aml_host::config::AppConfig::load();
        cfg.aes_key = Some(k.clone());
        let _ = cfg.save();
        self.key = Some(k);
        self.key_input.clear();
        self.status = "Key saved.".into();
        if self.live_path.is_some() {
            self.load();
        }
    }

    // (pak_from_running_game is a free fn below)

    /// Kick off background key recovery from the running game (non-blocking).
    fn start_recovery(&mut self) {
        if self.recovery.is_some() {
            return;
        }
        // Locate a pak to validate the key against. Prefer Steam detection, but
        // fall back to deriving it from the RUNNING game's exe path — that works
        // when Steam or the game live on a non-default folder or drive, or aren't
        // a Steam install at all (the game is running, so we have its path).
        let pak = aml_host::find_game()
            .ok()
            .map(|g| g.layout.paks_dir().join("pakchunk0-WindowsClient.pak"))
            .filter(|p| p.exists())
            .or_else(pak_from_running_game);
        let Some(pak) = pak else {
            self.note("Couldn't find the game's paks. Make sure Echoes of Aincrad is running and you're in the world, then try again.");
            return;
        };
        let pid = ks::find_game_pid();
        append_log(&format!("recovery start: pak={} game_pid={:?}", pak.display(), pid));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let res = ks::recover_key(&pak).map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
        self.recovery = Some(rx);
        self.note("Scanning the running game for your key… (this can take a moment)");
    }

    /// Poll the background recovery; apply the key when it finishes.
    fn poll_recovery(&mut self) {
        if let Some(rx) = &self.recovery {
            match rx.try_recv() {
                Ok(Ok(key)) => {
                    self.recovery = None;
                    self.key_input = key;
                    self.set_key();
                    self.note("Recovered your key from the running game.");
                }
                Ok(Err(e)) => {
                    self.recovery = None;
                    self.note(format!("Key recovery failed: {e}"));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => self.recovery = None,
            }
        }
    }

    /// Poll the background update check. It sends a message only if a newer
    /// release exists; either way the receiver disconnects when the thread ends,
    /// so we stop polling after the first non-empty result.
    fn poll_update(&mut self) {
        if let Some(rx) = &self.update_rx {
            match rx.try_recv() {
                Ok(u) => {
                    self.update = Some(u);
                    self.update_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_rx = None,
            }
        }
    }

    /// Copy the live save to the work copy and load that (never edit live directly).
    fn load(&mut self) {
        let Some(key) = self.key.clone() else { return };
        let Some(live) = self.live_path.clone() else { return };
        if let Some(parent) = self.work_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(&live, &self.work_path) {
            self.note(format!("Could not copy save to a work file: {e}"));
            return;
        }
        match SaveFile::load(&self.work_path, &key) {
            Ok(sf) => {
                let n = sf.character_count();
                // Keep the selected slot in range: loading a save with fewer
                // characters than the last one must not leave slot pointing past
                // the end (which would show an empty, uneditable character).
                self.slot = self.slot.min(n.saturating_sub(1));
                self.fields = sf.appearance(self.slot).unwrap_or_default();
                self.save = Some(sf);
                self.dirty = false;
                // Flag the global-scale bug (a non-1.0 MeshScale on any slot) so we
                // can offer a one-click fix; don't touch the save silently.
                self.scale_bug = self.detect_scale_bug(n);
                self.status =
                    format!("Loaded {n} character(s) into a working copy — your live save is untouched.");
            }
            Err(e) => {
                // A torn/locked save is the usual cause when the game is running.
                if ks::find_game_pid().is_some() {
                    self.note(format!(
                        "Couldn't read the save — close Echoes of Aincrad first (it's running and writing the save), then Reload. [{e}]"
                    ));
                } else {
                    self.note(format!("Failed to read save: {e}"));
                }
            }
        }
    }

    /// Switch to editing character `slot` and reload its fields.
    fn select_slot(&mut self, slot: usize) {
        if let Some(sf) = &self.save {
            if slot < sf.character_count() && slot != self.slot {
                self.slot = slot;
                self.fields = sf.appearance(slot).unwrap_or_default();
            }
        }
    }

    fn set(&mut self, name: &str, v: FieldValue) {
        if let Some(sf) = &mut self.save {
            if sf.set_appearance(self.slot, name, v).is_ok() {
                self.fields = sf.appearance(self.slot).unwrap_or_default();
                self.dirty = true;
            }
        }
    }

    /// The active language's UI strings.
    fn tr(&self) -> &'static S {
        i18n::s(self.lang)
    }

    /// Change UI language and persist it.
    fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
        let mut cfg = aml_host::config::AppConfig::load();
        cfg.lang = Some(lang.code().to_string());
        let _ = cfg.save();
    }

    /// Set the status line and append it to the diagnostics log.
    fn note(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        append_log(&msg);
        self.status = msg;
    }

    /// A shareable diagnostics blob: OS, version, save/key state, and the log tail.
    fn diagnostics(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("Aincrad Save Editor {}\n", env!("CARGO_PKG_VERSION")));
        s.push_str(&format!("os: {} {}\n", std::env::consts::OS, std::env::consts::ARCH));
        s.push_str(&format!("key set: {}\n", self.key.is_some()));
        s.push_str(&format!("save loaded: {} (slot {})\n", self.save.is_some(), self.slot));
        s.push_str(&format!(
            "live save: {}\n",
            self.live_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not found".into())
        ));
        s.push_str(&format!("game running: {}\n", ks::find_game_pid().is_some()));
        s.push_str(&format!("last status: {}\n", self.status));
        s.push_str("--- log tail ---\n");
        if let Ok(text) = std::fs::read_to_string(locate::log_path()) {
            for line in text.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
                s.push_str(line);
                s.push('\n');
            }
        }
        s
    }

    fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// True if any character slot has a non-1.0 `MeshScale` — the value that
    /// resizes every character/mob in-game to one height.
    fn detect_scale_bug(&self, n: usize) -> bool {
        let Some(sf) = &self.save else { return false };
        (0..n).any(|s| {
            sf.appearance(s).into_iter().flatten().any(|f| {
                f.name == "MeshScale"
                    && matches!(f.value, FieldValue::Float(v) if (v - 1.0).abs() > 1e-4)
            })
        })
    }

    /// The face's base skin layer (`CustomColorFaceG`) differs from the body skin
    /// (`CustomColorSkin`) — i.e. the face won't match the body. Small rounding is
    /// ignored.
    fn face_skin_mismatch(&self) -> bool {
        let skin = self.color("CustomColorSkin");
        let g = self.color("CustomColorFaceG");
        match (skin, g) {
            (Some(s), Some(g)) => (0..3).any(|i| (s[i] - g[i]).abs() > 3.0 / 255.0),
            _ => false,
        }
    }

    /// Read a colour field as rgba, if present.
    fn color(&self, name: &str) -> Option<[f32; 4]> {
        match self.field(name).map(|f| &f.value) {
            Some(FieldValue::Color(c)) => Some(*c),
            _ => None,
        }
    }

    /// Force the face to match the body skin: set the face base layer (`FaceG`)
    /// equal to the body skin, and shift the highlight (`FaceR`) by the same delta
    /// so it re-tints to the new base instead of snapping to a flat colour. `FaceB`
    /// (a dark detail line) is left alone. Repairs an already-mismatched save.
    fn match_face_to_skin(&mut self) {
        let Some(skin) = self.color("CustomColorSkin") else { return };
        let Some(g) = self.color("CustomColorFaceG") else { return };
        if let Some(sf) = &mut self.save {
            sf.note_edit("ran Match face to skin");
        }
        let d = [skin[0] - g[0], skin[1] - g[1], skin[2] - g[2]];
        if let Some(r) = self.color("CustomColorFaceR") {
            let nr = [
                (r[0] + d[0]).clamp(0.0, 1.0),
                (r[1] + d[1]).clamp(0.0, 1.0),
                (r[2] + d[2]).clamp(0.0, 1.0),
                r[3],
            ];
            self.set("CustomColorFaceR", FieldValue::Color(nr));
        }
        self.set("CustomColorFaceG", FieldValue::Color(skin));
        self.note("Matched the face to your skin tone. Click \"Apply to game\" to save it.");
    }

    /// Reset `MeshScale` to 1.0 on every slot (fixes the global-scale bug), marks
    /// the save dirty, and clears the flag. The user still confirms via "Apply".
    fn fix_scale(&mut self) {
        let n = self.save.as_ref().map(|s| s.character_count()).unwrap_or(0);
        if let Some(sf) = &mut self.save {
            sf.note_edit("ran Fix body scale (MeshScale -> 1.0 on all slots)");
            for s in 0..n {
                let _ = sf.set_appearance(s, "MeshScale", FieldValue::Float(1.0));
            }
            self.fields = sf.appearance(self.slot).unwrap_or_default();
        }
        self.scale_bug = false;
        self.dirty = true;
        self.note(
            "Reset body scale to normal on all characters. Click \"Apply to game\" to save the fix.",
        );
    }
    fn int(&self, name: &str) -> Option<i32> {
        match self.field(name)?.value {
            FieldValue::Int(v) => Some(v),
            _ => None,
        }
    }
    fn float(&self, name: &str) -> Option<f32> {
        match self.field(name)?.value {
            FieldValue::Float(v) => Some(v),
            _ => None,
        }
    }

    /// Rescan the looks folder for saved presets.
    fn scan_looks(&mut self) {
        let mut v: Vec<PathBuf> = std::fs::read_dir(locate::looks_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        v.sort();
        self.looks = v;
    }

    /// Capture the current character's appearance as a named look (JSON).
    fn save_look(&mut self) {
        let name = self.new_look_name.trim();
        if name.is_empty() {
            self.status = "Give the look a name first.".into();
            return;
        }
        let Some(sf) = &self.save else { return };
        match Look::capture(sf, self.slot, name).and_then(|l| l.to_json()) {
            Ok(json) => {
                let dir = locate::looks_dir();
                let _ = std::fs::create_dir_all(&dir);
                let safe: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                let path = dir.join(format!("{safe}.json"));
                match std::fs::write(&path, json) {
                    Ok(()) => {
                        self.status = format!("Saved look “{name}”.");
                        self.new_look_name.clear();
                        self.scan_looks();
                    }
                    Err(e) => self.status = format!("Could not save look: {e}"),
                }
            }
            Err(e) => self.status = format!("Could not capture look: {e}"),
        }
    }

    /// Apply a saved look to the current character (edits the working copy).
    fn apply_look(&mut self, path: &std::path::Path) {
        let look = match std::fs::read_to_string(path).ok().and_then(|s| Look::from_json(&s).ok()) {
            Some(l) => l,
            None => {
                self.status = "Could not read that look.".into();
                return;
            }
        };
        let slot = self.slot;
        if let Some(sf) = &mut self.save {
            let n = look.apply(sf, slot);
            self.fields = sf.appearance(slot).unwrap_or_default();
            self.dirty = true;
            self.status = format!("Applied look “{}” ({n} fields). Save or Apply to keep it.", look.name);
        }
    }

    /// Save edits into the work copy (with a timestamped backup of it).
    fn save_work(&mut self) {
        if let Some(sf) = &self.save {
            match sf.write(&self.work_path) {
                Ok(()) => {
                    self.dirty = false;
                    self.status = format!("Saved to working copy: {}", self.work_path.display());
                }
                Err(e) => {
                    self.note(format!("Save failed: {e}"));
                    self.raise_save_error(&e);
                }
            }
        }
        self.scan_backups();
    }

    /// Refresh the Backups page rows: the working copy's backups folder plus the
    /// live save's (next to the game's save file), newest first.
    fn scan_backups(&mut self) {
        let mut v = scan_backup_dir(&locate::work_backups_dir(), false);
        if let Some(live) = &self.live_path {
            if let Some(dir) = live.parent() {
                v.extend(scan_backup_dir(&dir.join("backups"), true));
            }
        }
        v.sort_by_key(|b| std::cmp::Reverse(b.ts));
        self.backups = v;
    }

    /// Load a backup into the working copy (backing the current work copy up
    /// first, so nothing is ever lost). The live save stays untouched until the
    /// user explicitly applies.
    fn load_backup(&mut self, path: &Path) {
        let Some(key) = self.key.clone() else { return };
        if let Err(e) = aml_save::backup(&self.work_path) {
            self.note(format!("Aborted — could not back up the current working copy: {e}"));
            return;
        }
        if let Some(parent) = self.work_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(path, &self.work_path) {
            self.note(format!("Could not copy the backup into the working copy: {e}"));
            return;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        match SaveFile::load(&self.work_path, &key) {
            Ok(sf) => {
                let n = sf.character_count();
                self.slot = self.slot.min(n.saturating_sub(1));
                self.fields = sf.appearance(self.slot).unwrap_or_default();
                self.save = Some(sf);
                self.scale_bug = self.detect_scale_bug(n);
                self.dirty = false;
                self.status = format!(
                    "Loaded backup {name} ({n} character(s)) into the working copy — inspect it, then “Apply to game…” to restore it. Your live save is untouched until then."
                );
            }
            Err(e) => {
                self.save = None;
                self.fields = Vec::new();
                self.note(format!("Backup {name} would not load: {e}"));
            }
        }
        self.scan_backups();
    }

    /// Stash a save failure as a full, copy-pasteable report (error + why + app/OS
    /// context) and open the modal so the user can send it to us — people won't
    /// relay an error unless it's one click to copy.
    fn raise_save_error(&mut self, e: &aml_save::SaveError) {
        let mut report = String::new();
        report.push_str(&format!("Aincrad Save Editor {} — save failed\n", env!("CARGO_PKG_VERSION")));
        report.push_str(&format!("os: {} {}\n\n", std::env::consts::OS, std::env::consts::ARCH));
        report.push_str(&e.to_string());
        self.save_error = Some(report);
    }

    /// Copy the (saved) work file over the live save, timestamp-backing it up first.
    fn apply_live(&mut self) {
        let Some(live) = self.live_path.clone() else { return };
        self.save_work();
        match aml_save::backup(&live) {
            Ok(bak) => {
                if let Err(e) = std::fs::copy(&self.work_path, &live) {
                    self.note(format!("Apply failed: {e}"));
                } else {
                    self.status = match bak {
                        Some(b) => format!("Applied to your live save. Backup: {}", b.display()),
                        None => "Applied to your live save.".into(),
                    };
                }
            }
            Err(e) => self.note(format!("Aborted — could not back up live save: {e}")),
        }
        self.confirm_live = false;
        self.scan_backups();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_recovery();
        self.poll_update();
        if self.recovery.is_some() || self.update_rx.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        top_bar(self, ui);
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).small().color(theme::SUBTEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(self.tr().copy_diagnostics)
                        .on_hover_text("Copy app/OS info + recent log to the clipboard — paste it to us if something doesn't work")
                        .clicked()
                    {
                        let diag = self.diagnostics();
                        ui.ctx().copy_text(diag);
                        self.status = "Diagnostics copied to clipboard.".into();
                    }
                });
            });
        });

        if self.key.is_none() {
            let t = self.tr();
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.heading(t.set_key);
                    ui.add_space(6.0);
                    ui.label(RichText::new(t.key_needs).color(theme::SUBTEXT));
                    ui.add_space(10.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.key_input)
                            .hint_text("0x…")
                            .desired_width(560.0),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let submit = ui.button(RichText::new(t.save_key).strong()).clicked();
                        if submit || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            self.set_key();
                        }
                        // Key recovery reads the running game's memory — keyscan build only.
                        #[cfg(feature = "keyscan")]
                        {
                            let recovering = self.recovery.is_some();
                            if ui
                                .add_enabled(!recovering, egui::Button::new(if recovering {
                                    t.key_scanning
                                } else {
                                    t.key_recover
                                }))
                                .on_hover_text("Launch the game (get into the world), then click to read your key from it")
                                .clicked()
                            {
                                self.start_recovery();
                            }
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(t.key_hint).size(15.0).color(theme::TEXT));
                    ui.label(RichText::new(t.key_not_ship).size(13.0).italics().color(theme::SUBTEXT));
                });
            });
            return;
        }

        if self.save.is_none() {
            // No loaded save. Still offer the backups list — restoring a backup
            // is MOST needed exactly when the current save won't load.
            let t = self.tr();
            egui::CentralPanel::default().show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(t.open_to_begin).italics().color(theme::SUBTEXT));
                    });
                    ui.add_space(16.0);
                    backups_page(self, ui);
                });
            });
            return;
        }

        // Global-scale bug: a non-1.0 MeshScale resizes every character/mob in-game.
        // Offer a one-click fix at the top so anyone who hit it can repair their save.
        if self.scale_bug {
            egui::Panel::top("scale_bug_banner").show_inside(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("⚠ This save has a modified body scale that resizes every character and mob in the game to one height.")
                            .strong()
                            .color(egui::Color32::from_rgb(240, 180, 60)),
                    );
                });
                ui.add_space(2.0);
                if ui.button(RichText::new("Fix character scale").strong()).clicked() {
                    self.fix_scale();
                }
                ui.add_space(6.0);
            });
        }

        // Left: category rail. Right: the selected category only — no endless scroll.
        egui::Panel::left("cats")
            .resizable(false)
            .default_size(160.0)
            .show_inside(ui, |ui| {
                ui.add_space(6.0);
                let t = self.tr();
                for &cat in CATEGORY_ORDER {
                    let selected = self.category == cat;
                    if ui
                        .add_sized([ui.available_width(), 30.0], egui::Button::selectable(selected, cat_label(t, cat)))
                        .clicked()
                    {
                        self.category = cat;
                        if cat == Category::Backups {
                            self.scan_backups();
                        }
                    }
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.category {
                Category::Identity => identity_page(self, ui),
                Category::Face => {
                    pickers_page(self, ui, FACE);
                    colours_page(self, ui, Category::Face);
                }
                Category::Hair => {
                    pickers_page(self, ui, HAIR);
                    colours_page(self, ui, Category::Hair);
                    if NPC_HAIR_SECTION_ENABLED {
                        npc_hair_mod_page(self, ui);
                    }
                }
                Category::Body => {
                    body_page(self, ui);
                    colours_page(self, ui, Category::Body);
                }
                Category::Looks => looks_page(self, ui),
                Category::Backups => backups_page(self, ui),
            });
        });
    }
}

/// The Backups page: every timestamped backup the editor has made (working copy
/// + live save), newest first, loadable back into the working copy.
fn backups_page(app: &mut App, ui: &mut egui::Ui) {
    let t = app.tr();
    ui.heading(t.cat_backups);
    ui.label(RichText::new(t.backups_intro).small().color(theme::SUBTEXT));
    ui.add_space(6.0);

    if app.backups.is_empty() {
        ui.label(RichText::new(t.no_backups).italics().color(theme::SUBTEXT));
        return;
    }
    let rows = app.backups.clone();
    let mut load: Option<PathBuf> = None;
    card(ui, |ui| {
        egui::Grid::new("backups").num_columns(4).spacing([14.0, 6.0]).show(ui, |ui| {
            for b in &rows {
                let when = chrono::DateTime::from_timestamp(b.ts, 0)
                    .map(|utc| {
                        utc.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_else(|| "?".into());
                ui.label(when);
                let src = if b.live { t.backup_live } else { t.backup_work };
                ui.label(RichText::new(src).small().color(theme::SUBTEXT));
                ui.label(
                    RichText::new(format!("{} KB", b.size / 1024)).small().color(theme::SUBTEXT),
                );
                let can = app.key.is_some();
                if ui
                    .add_enabled(can, egui::Button::new(t.backup_load))
                    .on_hover_text(b.path.display().to_string())
                    .clicked()
                {
                    load = Some(b.path.clone());
                }
                ui.end_row();
            }
        });
    });
    if let Some(p) = load {
        app.load_backup(&p);
    }
}

fn top_bar(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("top").show_inside(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading("Aincrad Save Editor");
            ui.add_space(8.0);
            let t = app.tr();
            theme::pill(ui, t.working_copy, theme::GREEN).on_hover_text(
                "Edits go to a copy; the game's save is only changed via “Apply to game”, which backs it up first.",
            );
            if app.dirty {
                theme::pill(ui, t.unsaved, theme::PEACH);
            }
            // Subtle "newer version available" link (only when the launch check
            // found one). Clicking opens the releases page in the browser.
            if let Some(u) = &app.update {
                ui.hyperlink_to(
                    RichText::new(format!("⬆ {} v{}", t.update_available, u.version)).color(theme::PEACH),
                    &u.url,
                )
                .on_hover_text(t.update_hint);
            }
            // Language selector.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cur = Lang::ALL.iter().find(|(l, _)| *l == app.lang).map(|(_, n)| *n).unwrap_or("English");
                egui::ComboBox::from_id_salt("lang").selected_text(cur).show_ui(ui, |ui| {
                    for (lang, native) in Lang::ALL {
                        if ui.selectable_label(app.lang == *lang, *native).clicked() {
                            app.set_lang(*lang);
                        }
                    }
                });
                ui.label(RichText::new(t.language).small().color(theme::SUBTEXT));
            });
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let t = app.tr();
            if ui.button(t.open_save).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("save", &["sav"]).pick_file() {
                    app.live_path = Some(p);
                    app.load();
                }
            }
            let has = app.save.is_some();
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(RichText::new(t.save).strong()).fill(theme::tint(theme::GREEN, 55)),
                )
                .clicked()
            {
                app.save_work();
            }
            if ui.add_enabled(has && app.live_path.is_some(), egui::Button::new(t.apply_to_game)).clicked() {
                app.confirm_live = true;
            }
            if ui.add_enabled(has, egui::Button::new(t.reload)).clicked() {
                app.load();
            }
            // Return to the key screen (paste a new key, or re-recover from the game).
            // Keeps recovery reachable after a key is stored — otherwise the button on
            // the key screen is unreachable once a key is saved.
            if ui
                .button(t.change_key)
                .on_hover_text("Go back to the key screen to paste a new key or recover it from the running game")
                .clicked()
            {
                app.key = None;
                app.key_input.clear();
            }
            // Character-slot selector (only when the save holds more than one).
            let count = app.save.as_ref().map(|s| s.character_count()).unwrap_or(0);
            if count > 1 {
                ui.separator();
                ui.label(t.character);
                if ui.add_enabled(app.slot > 0, egui::Button::new("◀")).clicked() {
                    app.select_slot(app.slot - 1);
                }
                ui.label(format!("{}/{}", app.slot + 1, count));
                if ui.add_enabled(app.slot + 1 < count, egui::Button::new("▶")).clicked() {
                    app.select_slot(app.slot + 1);
                }
            }
            ui.separator();
            ui.label(t.thumbnails);
            ui.add(egui::Slider::new(&mut app.thumb_scale, 0.75..=2.5).show_value(false))
                .on_hover_text("Make the picker thumbnails bigger / smaller");
        });
        ui.add_space(4.0);
    });

    if app.confirm_live {
        egui::Window::new("Apply to your live save?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label("This overwrites the game's actual save file.");
                ui.label(
                    RichText::new("A timestamped backup is made first (in a backups/ folder).")
                        .small()
                        .color(theme::SUBTEXT),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Apply").color(theme::RED)).clicked() {
                        app.apply_live();
                    }
                    if ui.button("Cancel").clicked() {
                        app.confirm_live = false;
                    }
                });
            });
    }

    if let Some(report) = app.save_error.clone() {
        let mut keep_open = true;
        egui::Window::new("Save failed — copy this and send it to us")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.set_max_width(580.0);
                ui.label(
                    RichText::new(
                        "Your save was NOT changed. Copy the report below and send it to the \
                         developer so this save can be supported.",
                    )
                    .color(theme::SUBTEXT),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    // A selectable/copyable read-only view (edits go to a throwaway copy).
                    let mut shown = report.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut shown)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Copy to clipboard").strong().color(theme::GREEN)).clicked() {
                        ui.ctx().copy_text(report.clone());
                        app.status = "Save-error report copied — paste it to us.".into();
                    }
                    if ui.button("Close").clicked() {
                        keep_open = false;
                    }
                });
            });
        if !keep_open {
            app.save_error = None;
        }
    }
}

fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
}

fn identity_page(app: &mut App, ui: &mut egui::Ui) {
    let t = app.tr();
    ui.heading(t.cat_identity);
    ui.add_space(4.0);
    let name = match app.field("HeroName") {
        Some(Field { value: FieldValue::Str(s), .. }) => s.clone(),
        _ => String::new(),
    };
    // Gender is an EnumProperty like "ECharacterSex::Male" — keep the enum prefix
    // when writing, and show just the short label ("Male"/"Female").
    let gender_full = match app.field("Gender") {
        Some(Field { value: FieldValue::Enum(g), .. }) => g.clone(),
        _ => String::new(),
    };
    let gender_short = gender_full.rsplit("::").next().unwrap_or("").to_string();
    let gender_prefix = gender_full
        .rfind("::")
        .map(|i| gender_full[..i + 2].to_string())
        .unwrap_or_else(|| "ECharacterSex::".to_string());
    let voice = match app.field("Voice") {
        Some(Field { value: FieldValue::Name(v), .. }) => v.clone(),
        _ => String::new(),
    };
    card(ui, |ui| {
        egui::Grid::new("identity").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label(t.name);
            let mut n = name.clone();
            if ui.text_edit_singleline(&mut n).changed() && n != name {
                app.set("HeroName", FieldValue::Str(n));
            }
            ui.end_row();
            if !gender_full.is_empty() {
                ui.label(t.gender);
                ui.horizontal(|ui| {
                    // Display the translated label; compare/write with the enum key.
                    for (key, label) in [("Male", t.male), ("Female", t.female)] {
                        if ui.selectable_label(gender_short == key, label).clicked() && gender_short != key {
                            app.set("Gender", FieldValue::Enum(format!("{gender_prefix}{key}")));
                            // The voice is deliberately KEPT: voices aren't
                            // gender-locked (the Wwise switch is keyed by the
                            // voice name alone), and auto-swapping would clobber
                            // a deliberate cross-gender pick.
                            app.note("Gender changed — voice kept (any voice works on any body). Double-check face/hair parts still suit the new body.");
                        }
                    }
                });
                ui.end_row();
            }
            if !voice.is_empty() {
                ui.label(t.voice);
                // All 12 voices visible at once — a male row and a female row.
                // The old ◀/▶ stepper crossed genders correctly but only ever
                // SHOWED the current one ("Male 3 / 6"), so nobody discovered
                // the other six voices past the end of their row.
                ui.vertical(|ui| {
                    for (group, list) in [(t.male, &MALE_VOICES), (t.female, &FEMALE_VOICES)] {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [52.0, 18.0],
                                egui::Label::new(RichText::new(group).small().color(theme::SUBTEXT)),
                            );
                            for (i, v) in list.iter().enumerate() {
                                if ui.selectable_label(voice == *v, format!("{}", i + 1)).clicked()
                                    && voice != *v
                                {
                                    app.set("Voice", FieldValue::Name(v.to_string()));
                                    // Mirror the in-game creator: picking a voice
                                    // speaks one of its sample lines right away.
                                    app.preview.play(v);
                                }
                            }
                        });
                    }
                    if app.preview.any() {
                        ui.horizontal(|ui| {
                            if ui.button("▶").on_hover_text(t.voice_preview).clicked() {
                                app.preview.play(&voice);
                            }
                            // Audio-language toggle, only when both dubs shipped.
                            use voice_preview::AudioLang;
                            if AudioLang::ALL.iter().all(|l| app.preview.lang_available(*l)) {
                                for l in AudioLang::ALL {
                                    if ui
                                        .selectable_label(app.preview.lang == l, RichText::new(l.label()).small())
                                        .clicked()
                                    {
                                        app.preview.lang = l;
                                    }
                                }
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label("");
                ui.label(RichText::new(t.voice_any_body).small().italics().color(theme::SUBTEXT));
                ui.end_row();
            }
        });
    });

    // Slot-level game mode: the character-creation permadeath flag. Lives on
    // the slot struct outside AvatarData (so looks can never carry it) and is
    // read live from the save tree, so it follows the character selector.
    let death_game = app.save.as_ref().and_then(|s| s.death_game_mode(app.slot).ok());
    if let Some(on) = death_game {
        ui.add_space(10.0);
        ui.heading(t.mode_title);
        ui.add_space(4.0);
        card(ui, |ui| {
            let mut b = on;
            if ui.checkbox(&mut b, t.death_game).changed() {
                if let Some(sf) = &mut app.save {
                    if sf.set_death_game_mode(app.slot, b).is_ok() {
                        app.dirty = true;
                    }
                }
            }
            ui.label(RichText::new(t.death_game_note).small().color(theme::SUBTEXT));
        });
    }
}

/// The **NPC side**: hairstyles the game defines in `DT_HeadGearParts` (the
/// `HG800xxx`/`HG85xxxx` series used by NPCs / special characters) but does NOT
/// offer in the character creator. Each has its own `HG*_Default` mesh parts, so
/// the game can render them on the player. Kept as their own list — separate from
/// the PC char-creator set in `PART_IDS` — so the UI has a clear PC side / NPC side.
/// No thumbnails exist for these; shown as numbered chips. Experimental (some may
/// suit only one gender); every Apply makes a backup.
const NPC_HAIR: &[i32] = &[
    800001, 801001, 801021, 802001, 803001, 804001, 805001, 806001, 807001, 807031,
    807502, 807504, 808001, 809001, 850001, 850505, 851001, 851503, 852001, 852011,
    853001, 854001, 854011, 855001, 856001, 856031, 857001,
];

/// PC-side valid ids (character-creator) for a part folder (empty if unknown).
/// The table lives in `aml-save` (`appearance::PART_IDS`) so the steppers here
/// and the preset-apply validation share one source; when thumbnails are
/// present, `available_ids` (read from the bundle) is the authoritative source.
fn part_ids_for(folder: &str) -> &'static [i32] {
    aml_save::appearance::PART_IDS
        .iter()
        .find(|p| p.folder == folder)
        .map(|p| p.ids)
        .unwrap_or(&[])
}

/// Whether to surface the NPC/extra hairs in the picker. OFF until the companion
/// "custom hair" mod ships: the base game maps `HeadGearID` to an index in a fixed
/// ~20-entry skeletal-mesh array, so an NPC id (800001 → index ~799) runs off the
/// end and CRASHES the game. The mod appends those meshes to the array, making the
/// ids valid; only then is it safe to flip this on.
const NPC_HAIR_ENABLED: bool = false;

/// Show the "NPC hairstyles" section (drives the hairswap UE4SS mod) on the Hair
/// tab. Hidden for now — the runtime hair swap isn't reliable yet and it was
/// confusing users. Flip to `true` to bring it back once hairswap is solid.
const NPC_HAIR_SECTION_ENABLED: bool = false;

/// NPC-side extra ids for a part folder (only hair has any today).
fn extra_ids_for(folder: &str) -> &'static [i32] {
    if NPC_HAIR_ENABLED && folder == "HeadGear" {
        NPC_HAIR
    } else {
        &[]
    }
}

/// Step `cur` to the previous/next valid id for a part folder, staying within the
/// real set (so we never write an id the game has no part for). Falls back to a
/// plain ±1 only for a folder we have no id list for. An unrecognized `cur` snaps
/// to the nearest valid id.
fn step_part_id(folder: &str, cur: i32, delta: i32) -> i32 {
    let ids = part_ids_for(folder);
    if ids.is_empty() {
        return cur + delta; // unknown part: no id list to stay within
    }
    match ids.iter().position(|&v| v == cur) {
        Some(i) => {
            let next = (i as i32 + delta).clamp(0, ids.len() as i32 - 1) as usize;
            ids[next]
        }
        None => *ids.iter().min_by_key(|&&v| (v - cur).abs()).unwrap_or(&cur),
    }
}

fn pickers_page(app: &mut App, ui: &mut egui::Ui, pickers: &[Picker]) {
    for p in pickers {
        let Some(cur) = app.int(p.field) else { continue };
        ui.heading(picker_label(app.tr(), p.field));
        ui.add_space(3.0);
        card(ui, |ui| {
            let ids = app.thumbs.available_ids(p.folder);
            let npc = extra_ids_for(p.folder);
            let mut pick = None;
            // --- PC side: the character-creator options ---
            if !npc.is_empty() {
                ui.label(RichText::new("Character creator").small().strong().color(theme::SUBTEXT));
            }
            if ids.is_empty() {
                // No thumbnails: numbered stepper over the PC ids.
                ui.horizontal(|ui| {
                    if ui.small_button("◀").clicked() {
                        pick = Some(step_part_id(p.folder, cur, -1));
                    }
                    ui.label(format!("#{cur}"));
                    if ui.small_button("▶").clicked() {
                        pick = Some(step_part_id(p.folder, cur, 1));
                    }
                });
                ui.label(
                    RichText::new("Run scripts/extract-thumbnails.py to see the pictures.")
                        .small()
                        .italics()
                        .color(theme::SUBTEXT),
                );
            } else {
                let px = 72.0 * app.thumb_scale;
                let none_label = app.tr().none;
                ui.horizontal_wrapped(|ui| {
                    if p.optional && thumbs::none_button(ui, cur == 0, none_label).clicked() {
                        pick = Some(0);
                    }
                    for &id in &ids {
                        if thumbs::thumb_button(ui, &mut app.thumbs, p.folder, id, id == cur, px).clicked() {
                            pick = Some(id);
                        }
                    }
                });
            }
            // --- NPC side: styles not in the char creator, as numbered chips ---
            if !npc.is_empty() {
                ui.add_space(8.0);
                ui.separator();
                ui.label(
                    RichText::new("NPC / extra styles (no preview — experimental)")
                        .small()
                        .strong()
                        .color(theme::SUBTEXT),
                );
                ui.horizontal_wrapped(|ui| {
                    for &id in npc {
                        if ui.selectable_label(id == cur, format!("#{id}")).clicked() {
                            pick = Some(id);
                        }
                    }
                });
            }
            if let Some(id) = pick {
                app.set(p.field, FieldValue::Int(id));
            }
        });
        ui.add_space(8.0);
    }
}

/// NPC-only hairstyles applied at runtime by the `hairswap` UE4SS mod. The
/// player's own hair lives in the save (edited above); these NPC styles can't (the
/// game nulls a redirected part and crashes), so we write the pick to the mod's
/// config file and it sets the mesh live. Hidden unless the hairswap mod is found.
fn npc_hair_mod_page(app: &mut App, ui: &mut egui::Ui) {
    let Some(cfg) = app.hairswap_cfg.clone() else { return };
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.heading("NPC hairstyles");
        theme::pill(ui, "hairswap mod", theme::MAUVE)
            .on_hover_text("Applied live by the hairswap UE4SS mod — not stored in the save.");
    });
    ui.label(
        RichText::new(
            "26 hairstyles only NPCs wear. These can't be written to your character file — the \
             hairswap mod puts the chosen one on you in-game. Takes effect on your next zone load \
             or game launch; in-game you can also cycle with Ctrl+Shift+Y.",
        )
        .small()
        .color(theme::SUBTEXT),
    );
    ui.add_space(6.0);

    // Some(Some(id)) = pick that id; Some(None) = clear the override.
    let mut choose: Option<Option<u32>> = None;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let cur = match app.npc_hair {
                Some(id) => format!("Current: #{id}"),
                None => "Current: your own hair".to_string(),
            };
            ui.label(RichText::new(cur).strong().color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(app.npc_hair.is_some(), egui::Button::new("Clear"))
                    .on_hover_text("Remove the override — the game uses your real hair.")
                    .clicked()
                {
                    choose = Some(None);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for &id in npchair::NPC_HAIR_IDS {
                if ui.selectable_label(app.npc_hair == Some(id), format!("#{id}")).clicked() {
                    choose = Some(Some(id));
                }
            }
        });
    });

    if let Some(pick) = choose {
        match npchair::write(&cfg, pick) {
            Ok(()) => {
                app.npc_hair = pick;
                match pick {
                    Some(id) => app.note(format!(
                        "NPC hair #{id} set for the hairswap mod — load a zone or relaunch to see it."
                    )),
                    None => app.note("Cleared the NPC-hair override — the game will use your own hair."),
                }
            }
            Err(e) => app.note(format!("Couldn't write the hairswap config: {e}")),
        }
    }
}

fn looks_page(app: &mut App, ui: &mut egui::Ui) {
    let t = app.tr();
    ui.heading(t.cat_looks);
    ui.label(RichText::new(t.looks_intro).small().color(theme::SUBTEXT));
    ui.add_space(6.0);

    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(t.save_look_as);
            ui.text_edit_singleline(&mut app.new_look_name);
            let can = app.save.is_some() && !app.new_look_name.trim().is_empty();
            if ui
                .add_enabled(can, egui::Button::new(t.save_look).fill(theme::tint(theme::GREEN, 55)))
                .clicked()
            {
                app.save_look();
            }
        });
    });
    ui.add_space(8.0);

    ui.label(RichText::new(t.saved_looks).strong());
    ui.add_space(3.0);
    if app.looks.is_empty() {
        ui.label(RichText::new(t.no_looks).italics().color(theme::SUBTEXT));
        return;
    }
    let looks = app.looks.clone();
    let mut apply: Option<PathBuf> = None;
    let mut delete: Option<PathBuf> = None;
    card(ui, |ui| {
        egui::Grid::new("looks").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
            for path in &looks {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                ui.label(name);
                if ui.add_enabled(app.save.is_some(), egui::Button::new(t.apply)).clicked() {
                    apply = Some(path.clone());
                }
                if ui.small_button(t.delete).clicked() {
                    delete = Some(path.clone());
                }
                ui.end_row();
            }
        });
    });
    if let Some(p) = apply {
        app.apply_look(&p);
    }
    if let Some(p) = delete {
        let _ = std::fs::remove_file(&p);
        app.scan_looks();
    }
}

fn body_page(app: &mut App, ui: &mut egui::Ui) {
    let t = app.tr();
    ui.heading(t.cat_body);
    ui.add_space(4.0);
    // The slider list and its safe caps come from `appearance::FLOAT_RANGES`
    // (one source with preset validation). Pinned entries (lo == hi, i.e.
    // MeshScale) aren't sliders: the creator never exposes MeshScale, and a
    // drifted value resizes every character and mob in the game — loading such
    // a save flags `scale_bug` and offers the one-click `fix_scale` instead.
    // Only show sliders for fields actually present in this save.
    let present: Vec<(&'static str, f32, f32, f32)> = aml_save::appearance::FLOAT_RANGES
        .iter()
        .filter(|&&(_, lo, hi)| lo < hi)
        .filter_map(|&(name, lo, hi)| app.float(name).map(|v| (name, lo, hi, v)))
        .collect();
    if present.is_empty() {
        return;
    }
    card(ui, |ui| {
        ui.label(RichText::new(t.body_hidden).small().color(theme::SUBTEXT));
        ui.label(RichText::new(t.body_neck_note).small().color(theme::SUBTEXT));
        ui.add_space(6.0);
        egui::Grid::new("body").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            for (name, lo, hi, cur) in present {
                ui.label(pretty(name));
                let mut v = cur;
                if ui.add(egui::Slider::new(&mut v, lo..=hi).fixed_decimals(2)).changed() {
                    // The slider already clamps; clamp again so the written value can
                    // never exceed the safe cap even via keyboard entry.
                    app.set(name, FieldValue::Float(v.clamp(lo, hi)));
                }
                ui.end_row();
            }
        });
    });
}

/// The colours (and their "use default" toggles) that belong to `cat`, rendered
/// beneath that category's parts so hair colours sit with hair, skin with body, etc.
fn colours_page(app: &mut App, ui: &mut egui::Ui, cat: Category) {
    let colors: Vec<(String, [f32; 4])> = app
        .fields
        .iter()
        .filter(|f| f.group == Group::Color && colour_category(&f.name) == cat)
        .filter_map(|f| match &f.value {
            FieldValue::Color(c) => Some((f.name.clone(), *c)),
            _ => None,
        })
        .collect();
    let toggles: Vec<(String, bool)> = app
        .fields
        .iter()
        .filter(|f| f.group == Group::Toggle && colour_category(&f.name) == cat)
        .filter_map(|f| match &f.value {
            FieldValue::Bool(b) => Some((f.name.clone(), *b)),
            _ => None,
        })
        .collect();
    if colors.is_empty() && toggles.is_empty() {
        return;
    }
    let t = app.tr();
    ui.heading(t.colours);
    ui.add_space(3.0);
    card(ui, |ui| {
        // One-click repair for the common "face doesn't match my skin tone" case:
        // the face's base layer drifted from the body skin. Shown on the Face/Body
        // colour pages (where skin + face colours live) when they actually differ.
        if matches!(cat, Category::Face | Category::Body) && app.face_skin_mismatch() {
            if ui
                .button(RichText::new("Match face to skin tone").strong())
                .on_hover_text("Your face colour doesn't match your body skin — this snaps it back so they match")
                .clicked()
            {
                app.match_face_to_skin();
            }
            ui.add_space(6.0);
        }
        for (name, mut b) in toggles {
            // "Use the game's default colour for X" — when on, the custom colour is ignored.
            if ui.checkbox(&mut b, format!("Default {} colour", pretty_default(&name))).changed() {
                app.set(&name, FieldValue::Bool(b));
            }
        }
        if !colors.is_empty() {
            ui.add_space(2.0);
            egui::Grid::new(("colors", cat as u8)).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                for (name, c) in &colors {
                    ui.label(pretty(name));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let mut rgba = *c;
                            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                                // Keep each channel a sane, finite [0,1] value so a stray
                                // keyboard entry can't write an out-of-gamut / NaN colour.
                                let clean =
                                    rgba.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 });
                                // Skin tone drives the face's skin layers so the face stays
                                // matched to the body. The face is three colours: FaceG is the
                                // base skin (equals the body skin), FaceR a lighter highlight.
                                // Shift both by the same delta as the skin change to preserve
                                // each layer's relationship (FaceB, a dark detail, is left alone).
                                // Without this, changing skin leaves a mismatched face — the
                                // #1 thing people hit.
                                if name == "CustomColorSkin" {
                                    let d = [clean[0] - c[0], clean[1] - c[1], clean[2] - c[2]];
                                    for face in ["CustomColorFaceG", "CustomColorFaceR"] {
                                        if let Some(FieldValue::Color(fc)) =
                                            app.field(face).map(|f| f.value.clone())
                                        {
                                            let nc = [
                                                (fc[0] + d[0]).clamp(0.0, 1.0),
                                                (fc[1] + d[1]).clamp(0.0, 1.0),
                                                (fc[2] + d[2]).clamp(0.0, 1.0),
                                                fc[3],
                                            ];
                                            app.set(face, FieldValue::Color(nc));
                                        }
                                    }
                                }
                                app.set(name, FieldValue::Color(clean));
                            }
                            if aml_save::palette::palette_for(name).is_some() {
                                let mut open = app.swatches_open.contains(name.as_str());
                                if ui
                                    .toggle_value(&mut open, RichText::new(t.game_palette).small())
                                    .on_hover_text(t.game_palette_hint)
                                    .changed()
                                {
                                    if open {
                                        app.swatches_open.insert(name.clone());
                                    } else {
                                        app.swatches_open.remove(name.as_str());
                                    }
                                }
                            }
                        });
                        if app.swatches_open.contains(name.as_str()) {
                            if let Some(pal) = aml_save::palette::palette_for(name) {
                                swatch_strip(app, ui, name, c, pal);
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            if colors.iter().any(|(n, _)| n == "CustomColorSkin") {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Skin tone also re-tints the face so they stay matched.")
                        .size(12.0)
                        .italics()
                        .color(theme::SUBTEXT),
                );
            }
        }
    });
}

/// A wrapped strip of the character creator's own swatches for `field`.
/// Each square previews exactly what a click writes to THIS field (sub-layer
/// fields take the row's SubColor), and clicking applies the creator's full
/// layer wiring via `palette::swatch_writes` — so a skin pick re-tints the
/// face layers the same way the in-game creator does. The palettes' row names
/// don't describe their colours (dev leftovers), so tooltips show the swatch
/// number + sRGB hex instead.
fn swatch_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    field: &str,
    current: &[f32; 4],
    pal: &'static [aml_save::palette::Swatch],
) {
    ui.set_max_width(324.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (i, s) in pal.iter().enumerate() {
            let target = aml_save::palette::swatch_writes(field, s)
                .into_iter()
                .find(|(n, _)| n == field)
                .map(|(_, c)| c)
                .unwrap_or([s.main[0], s.main[1], s.main[2], 1.0]);
            let selected = current.iter().zip(&target).all(|(a, b)| (a - b).abs() < 1e-4);
            // Save colours are linear; Color32::from(Rgba) applies the gamma the
            // game's own materials do, so the square matches the in-game shade.
            let col = egui::Color32::from(egui::Rgba::from_rgb(target[0], target[1], target[2]));
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            let stroke = if selected {
                egui::Stroke::new(2.0, theme::TEXT)
            } else if resp.hovered() {
                egui::Stroke::new(1.0, theme::OVERLAY)
            } else {
                egui::Stroke::new(1.0, theme::SURFACE2)
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 3.0, col);
            painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
            let hex = format!("#{:02X}{:02X}{:02X}", col.r(), col.g(), col.b());
            if resp.on_hover_text(format!("{} · {hex}", i + 1)).clicked() {
                for (n, c) in aml_save::palette::swatch_writes(field, s) {
                    app.set(&n, FieldValue::Color(c));
                }
            }
        }
    });
}

/// "bDefaultHairColor" -> "hair".
fn pretty_default(name: &str) -> String {
    name.strip_prefix("bDefault")
        .unwrap_or(name)
        .strip_suffix("Color")
        .unwrap_or(name)
        .to_string()
}

/// Append a line to the diagnostics log (best-effort; ignores errors).
fn append_log(msg: &str) {
    let path = locate::log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// Find a pak to validate the key against by walking up from the RUNNING game's
/// executable to its `Content/Paks` folder. Works regardless of where Steam or
/// the game are installed (any folder or drive, Steam or not). Prefers the known
/// `pakchunk0-WindowsClient.pak`, else any `.pak` in that folder.
fn pak_from_running_game() -> Option<PathBuf> {
    let exe = ks::find_game_exe()?;
    for dir in exe.ancestors() {
        let paks = dir.join("Content").join("Paks");
        if paks.is_dir() {
            let known = paks.join("pakchunk0-WindowsClient.pak");
            if known.is_file() {
                return Some(known);
            }
            return std::fs::read_dir(&paks)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().map(|x| x.eq_ignore_ascii_case("pak")).unwrap_or(false));
        }
    }
    None
}

// The valid character voices live in `aml_save::appearance` — single source
// for the voice picker AND preset validation. Voice 1 is the BARE "Player_M" /
// "Player_F" (no "_01", nothing above "_06") — the picker only ever offers
// these exact names, so it can't write an id the game has no audio for.
use aml_save::appearance::{FEMALE_VOICES, MALE_VOICES};

/// "CustomColorHairR" -> "Hair R", "MeshScale" -> "Mesh Scale".
fn pretty(name: &str) -> String {
    let n = name.strip_prefix("CustomColor").unwrap_or(name);
    let mut out = String::new();
    for (i, ch) in n.chars().enumerate() {
        if ch.is_uppercase() && i != 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod backup_tests {
    use super::*;

    #[test]
    fn ts_parses_from_backup_names() {
        // Both backup families use `<save-name>.<unix-seconds>.bak`.
        assert_eq!(backup_ts_from_name("SaveData.work.sav.1783726101.bak"), Some(1783726101));
        assert_eq!(backup_ts_from_name("SaveData.sav.1720000000.bak"), Some(1720000000));
        assert_eq!(backup_ts_from_name("SaveData.sav.bak"), None);
        assert_eq!(backup_ts_from_name("SaveData.sav"), None);
    }

    #[test]
    fn scan_finds_only_bak_files_and_sorting_is_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "SaveData.work.sav.100.bak",
            "SaveData.work.sav.300.bak",
            "SaveData.work.sav.200.bak",
            "SaveData.work.sav", // not a backup — must be skipped
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let mut v = scan_backup_dir(dir.path(), false);
        assert_eq!(v.len(), 3);
        v.sort_by_key(|b| std::cmp::Reverse(b.ts));
        let ts: Vec<i64> = v.iter().map(|b| b.ts).collect();
        assert_eq!(ts, [300, 200, 100]);
        assert!(v.iter().all(|b| !b.live));
    }

    #[test]
    fn scan_of_missing_dir_is_empty_not_error() {
        let v = scan_backup_dir(Path::new("/nonexistent/backups"), true);
        assert!(v.is_empty());
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;

    #[test]
    fn picker_offers_all_twelve_voices_no_gaps() {
        // The chip rows must expose every shipped voice (6 male + 6 female) and
        // only real ids: voice 1 is the BARE name (no "_01"), nothing above _06.
        // Every chip writes a name from these lists verbatim, so the picker
        // can't produce an id the game has no audio asset for.
        assert_eq!(MALE_VOICES.len(), 6);
        assert_eq!(FEMALE_VOICES.len(), 6);
        assert_eq!(MALE_VOICES[0], "Player_M");
        assert_eq!(FEMALE_VOICES[0], "Player_F");
        assert!(MALE_VOICES.iter().all(|v| !v.ends_with("_01")));
        assert!(FEMALE_VOICES.iter().all(|v| !v.ends_with("_01")));
        // The two rows never overlap (a chip selects exactly one save value).
        assert!(MALE_VOICES.iter().all(|v| !FEMALE_VOICES.contains(v)));
    }
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    #[test]
    fn hair_steps_by_1000_not_1() {
        assert_eq!(step_part_id("HeadGear", 3001, 1), 4001);
        assert_eq!(step_part_id("HeadGear", 3001, -1), 2001);
    }

    #[test]
    fn hair_clamps_at_ends() {
        // The ◀/▶ stepper covers the PC (char-creator) hairs; NPC styles are
        // separate chips, so the stepper clamps at the last creator hair.
        assert_eq!(step_part_id("HeadGear", 1001, -1), 1001);
        assert_eq!(step_part_id("HeadGear", 20001, 1), 20001);
    }

    #[test]
    fn npc_hair_is_separate_from_creator_set() {
        let pc = part_ids_for("HeadGear");
        assert_eq!(pc.len(), 20);
        assert!(!NPC_HAIR.is_empty());
        assert!(NPC_HAIR.iter().all(|id| !pc.contains(id))); // no overlap: clean PC/NPC split
        assert!(NPC_HAIR.contains(&800001));
        // Gated off until the custom-hair mod makes these ids safe (they crash the
        // base game). extra_ids_for stays empty while the flag is off.
        assert!(extra_ids_for("HeadGear").is_empty());
    }

    #[test]
    fn face_part_skips_gaps() {
        assert_eq!(step_part_id("Eyebrow", 12, 1), 14);
        assert_eq!(step_part_id("Eyebrow", 14, -1), 12);
        assert_eq!(step_part_id("Jaw", 25, 1), 30);
    }

    #[test]
    fn invalid_id_snaps_to_nearest_valid() {
        assert_eq!(step_part_id("HeadGear", 3000, 1), 3001);
        assert_eq!(step_part_id("Eyebrow", 13, 1), 12);
    }

    #[test]
    fn unknown_folder_falls_back_to_plus_minus_one() {
        assert_eq!(step_part_id("Unknown", 5, 1), 6);
    }
}
