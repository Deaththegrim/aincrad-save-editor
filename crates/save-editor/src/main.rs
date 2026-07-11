//! aml-save-editor — a friendly visual editor for Echoes of Aincrad character
//! appearance. Decrypts the save (via aml-save), shows the real in-game part
//! thumbnails so players pick a face/hair/eyes by sight rather than by number,
//! and writes changes back safely (work copy first, live save only on confirm,
//! always with a timestamped backup).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod i18n;
mod locate;
mod thumbs;

use aml_save::appearance::{Field, FieldValue, Group};
use aml_save::preset::Look;
use aml_save::SaveFile;
use aml_ui::theme;
use egui::RichText;
use i18n::{Lang, S};
use std::path::PathBuf;

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_title("Aincrad Save Editor"),
        ..Default::default()
    };
    eframe::run_native(
        "Aincrad Save Editor",
        opts,
        Box::new(|cc| {
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
}
const CATEGORY_ORDER: &[Category] = &[
    Category::Identity,
    Category::Face,
    Category::Hair,
    Category::Body,
    Category::Looks,
];

fn cat_label(t: &S, cat: Category) -> &'static str {
    match cat {
        Category::Identity => t.cat_identity,
        Category::Face => t.cat_face,
        Category::Hair => t.cat_hair,
        Category::Body => t.cat_body,
        Category::Looks => t.cat_looks,
    }
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
    lang: Lang,
}

impl App {
    fn new() -> Self {
        let cfg = aml_host::config::AppConfig::load();
        let key = cfg.aes_key;
        let lang = cfg.lang.as_deref().map(Lang::from_code).unwrap_or(Lang::En);
        let live_path = locate::find_save();
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
            lang,
        };
        app.scan_looks();
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
        let pid = aml_keyscan::find_game_pid();
        append_log(&format!("recovery start: pak={} game_pid={:?}", pak.display(), pid));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let res = aml_keyscan::recover_key(&pak).map_err(|e| e.to_string());
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
                self.status =
                    format!("Loaded {n} character(s) into a working copy — your live save is untouched.");
            }
            Err(e) => {
                // A torn/locked save is the usual cause when the game is running.
                if aml_keyscan::find_game_pid().is_some() {
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
        s.push_str(&format!("game running: {}\n", aml_keyscan::find_game_pid().is_some()));
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
                Err(e) => self.note(format!("Save failed: {e}")),
            }
        }
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
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_recovery();
        if self.recovery.is_some() {
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
                    });
                    ui.add_space(10.0);
                    ui.label(RichText::new(t.key_hint).size(15.0).color(theme::TEXT));
                    ui.label(RichText::new(t.key_not_ship).size(13.0).italics().color(theme::SUBTEXT));
                });
            });
            return;
        }

        if self.save.is_none() {
            let t = self.tr();
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(t.open_to_begin).italics().color(theme::SUBTEXT));
                });
            });
            return;
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
                }
                Category::Body => {
                    body_page(self, ui);
                    colours_page(self, ui, Category::Body);
                }
                Category::Looks => looks_page(self, ui),
            });
        });
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
                            // Keep the voice consistent: the game only offers
                            // gender-matching voices, so Player_M_06 <-> Player_F_06.
                            if let Some(FieldValue::Name(v)) = app.field("Voice").map(|f| f.value.clone()) {
                                let fixed = if key == "Female" {
                                    v.replacen("Player_M", "Player_F", 1)
                                } else {
                                    v.replacen("Player_F", "Player_M", 1)
                                };
                                if fixed != v {
                                    app.set("Voice", FieldValue::Name(fixed));
                                }
                            }
                            app.note("Gender changed — voice matched to it. Double-check face/hair parts still suit the new body.");
                        }
                    }
                });
                ui.end_row();
            }
            if !voice.is_empty() {
                ui.label(t.voice);
                ui.horizontal(|ui| {
                    // Show the 1-based position ("3 / 6") when recognized; the raw
                    // id otherwise (so an unexpected value is still visible).
                    let shown = match voice_index(&voice) {
                        Some(i) => format!("{} / {}", i + 1, voice_list(&voice).len()),
                        None => voice.clone(),
                    };
                    ui.label(RichText::new(shown).color(theme::SUBTEXT));
                    if ui.small_button("◀").clicked() {
                        if let Some(v) = step_voice(&voice, -1) {
                            app.set("Voice", FieldValue::Name(v));
                        }
                    }
                    if ui.small_button("▶").clicked() {
                        if let Some(v) = step_voice(&voice, 1) {
                            app.set("Voice", FieldValue::Name(v));
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
}

/// The valid part IDs the game's character creator offers, per thumbnail folder.
/// These are the SAVE ids (what the game stores), and they are NOT contiguous —
/// face parts skip numbers, and hair (HeadGear) steps by 1000 (1001, 2001, …).
/// Used to keep the no-thumbnail fallback stepper on real ids; when thumbnails are
/// present, `available_ids` (read from the bundle) is the authoritative source.
const PART_IDS: &[(&str, &[i32])] = &[
    ("Nose", &[1, 2, 3, 4, 5, 6, 7, 8]),
    ("Eyebrow", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 18, 21, 22, 27, 28, 29]),
    ("Eyeline", &[1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 17, 19, 20, 22, 23, 24, 27, 28, 29, 33, 34]),
    ("Pupil", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
    ("Jaw", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 30, 31, 32, 33, 34, 35, 36, 37, 38]),
    ("HeadGear", &[1001, 2001, 3001, 4001, 5001, 6001, 7001, 8001, 9001, 10001, 11001, 12001, 13001, 14001, 15001, 16001, 17001, 18001, 19001, 20001]),
    ("Mole", &[0, 1, 2, 3, 4, 5, 6, 7]),
    ("Freckles", &[0, 1, 2]),
];

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
fn part_ids_for(folder: &str) -> &'static [i32] {
    PART_IDS.iter().find(|(f, _)| *f == folder).map(|(_, v)| *v).unwrap_or(&[])
}

/// NPC-side extra ids for a part folder (only hair has any today).
fn extra_ids_for(folder: &str) -> &'static [i32] {
    if folder == "HeadGear" {
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

/// Body-shape sliders and their safe caps: `(save field, min, max)`.
///
/// The 10 morph weights run -1.0..=1.0 — the game's own char-creator range,
/// confirmed from the WBP_AvatarCustomize slider blueprints; outside it the
/// morphs extrapolate and warp the mesh. `MeshScale` (overall body scale) isn't a
/// body-panel slider, so it gets a conservative ±15% cap to avoid grotesque
/// resizing. Clamping to these is what keeps a user from breaking their model.
const BODY_SLIDERS: &[(&str, f32, f32)] = &[
    ("MeshScale", 0.85, 1.15),
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
];

fn body_page(app: &mut App, ui: &mut egui::Ui) {
    let t = app.tr();
    ui.heading(t.cat_body);
    ui.add_space(4.0);
    // Only show sliders for fields actually present in this save.
    let present: Vec<(&'static str, f32, f32, f32)> = BODY_SLIDERS
        .iter()
        .filter_map(|&(name, lo, hi)| app.float(name).map(|v| (name, lo, hi, v)))
        .collect();
    if present.is_empty() {
        return;
    }
    card(ui, |ui| {
        ui.label(RichText::new(t.body_hidden).small().color(theme::SUBTEXT));
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
    ui.heading(app.tr().colours);
    ui.add_space(3.0);
    card(ui, |ui| {
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
                    let mut rgba = *c;
                    if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                        // Keep each channel a sane, finite [0,1] value so a stray
                        // keyboard entry can't write an out-of-gamut / NaN colour.
                        let clean = rgba.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 });
                        app.set(name, FieldValue::Color(clean));
                    }
                    ui.end_row();
                }
            });
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
    let exe = aml_keyscan::find_game_exe()?;
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

/// The valid character voices, in order, per gender. Echoes of Aincrad ships
/// exactly 6 voices each. Note the quirk: voice 1 is the BARE "Player_M" /
/// "Player_F" (no number), and voices 2-6 are "_02".."_06" — there is no "_01",
/// and nothing above "_06". (Confirmed against the game's Switch_Avatar_Voice
/// assets.) The old code stepped the trailing number blindly and produced
/// "Player_M_07" / "Player_M_01", which the game has no asset for and silently
/// ignores — that was the "voice won't change" bug.
const MALE_VOICES: [&str; 6] =
    ["Player_M", "Player_M_02", "Player_M_03", "Player_M_04", "Player_M_05", "Player_M_06"];
const FEMALE_VOICES: [&str; 6] =
    ["Player_F", "Player_F_02", "Player_F_03", "Player_F_04", "Player_F_05", "Player_F_06"];

/// Which voice list applies, picked by the voice's own gender prefix.
fn voice_list(voice: &str) -> &'static [&'static str] {
    if voice.starts_with("Player_F") {
        &FEMALE_VOICES
    } else {
        &MALE_VOICES
    }
}

/// 1-based position of a voice within its list, for display ("3 / 6").
fn voice_index(voice: &str) -> Option<usize> {
    voice_list(voice).iter().position(|v| *v == voice)
}

/// Step to the previous/next valid voice, clamped to the real set so we never
/// write an id the game lacks. An unrecognized value (e.g. an invalid id written
/// by an older build) is repaired to the first valid voice on any step.
fn step_voice(voice: &str, delta: i32) -> Option<String> {
    let list = voice_list(voice);
    match list.iter().position(|v| *v == voice) {
        Some(cur) => {
            let next = (cur as i32 + delta).clamp(0, list.len() as i32 - 1) as usize;
            (next != cur).then(|| list[next].to_string())
        }
        None => Some(list[0].to_string()),
    }
}

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
mod voice_tests {
    use super::*;

    #[test]
    fn max_male_voice_stops_at_top_not_invalid() {
        // VEX's real save value. Stepping up must NOT produce "Player_M_07".
        assert_eq!(step_voice("Player_M_06", 1), None);
        assert_eq!(step_voice("Player_M_06", -1).as_deref(), Some("Player_M_05"));
    }

    #[test]
    fn voice_two_steps_down_to_bare_first_not_underscore_01() {
        // Voice 1 is the bare id; there is no "Player_M_01".
        assert_eq!(step_voice("Player_M_02", -1).as_deref(), Some("Player_M"));
        assert_eq!(step_voice("Player_M", -1), None); // already first
        assert_eq!(step_voice("Player_M", 1).as_deref(), Some("Player_M_02"));
    }

    #[test]
    fn female_list_used_for_female_voice() {
        assert_eq!(step_voice("Player_F", 1).as_deref(), Some("Player_F_02"));
        assert_eq!(step_voice("Player_F_06", 1), None);
    }

    #[test]
    fn invalid_id_repairs_to_first_valid() {
        // An id an older buggy build might have written.
        assert_eq!(step_voice("Player_M_07", -1).as_deref(), Some("Player_M"));
        assert_eq!(step_voice("Player_M_01", 1).as_deref(), Some("Player_M"));
    }

    #[test]
    fn index_is_one_based_within_six() {
        assert_eq!(voice_index("Player_M"), Some(0));
        assert_eq!(voice_index("Player_M_06"), Some(5));
        assert_eq!(voice_list("Player_M").len(), 6);
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
        let npc = extra_ids_for("HeadGear");
        assert_eq!(pc.len(), 20);
        assert!(!npc.is_empty());
        assert!(npc.iter().all(|id| !pc.contains(id))); // no overlap: clean PC/NPC split
        assert!(npc.contains(&800001));
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
