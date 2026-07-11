//! Loading + caching the extracted part thumbnails (`<dir>/<Part>/<id>.png`).

use egui::TextureHandle;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ThumbCache {
    dir: PathBuf,
    /// (part, id) -> GPU texture, loaded lazily and kept for the session.
    cache: HashMap<(String, i32), Option<TextureHandle>>,
    /// part -> sorted list of ids that have a PNG on disk (scanned once).
    ids: HashMap<String, Vec<i32>>,
}

impl ThumbCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, cache: HashMap::new(), ids: HashMap::new() }
    }

    /// The ids (sorted) that have a thumbnail for this part, scanning the folder once.
    pub fn available_ids(&mut self, part: &str) -> Vec<i32> {
        if let Some(v) = self.ids.get(part) {
            return v.clone();
        }
        let mut ids: Vec<i32> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.dir.join(part)) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "png") {
                    if let Some(n) = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok()) {
                        ids.push(n);
                    }
                }
            }
        }
        ids.sort_unstable();
        self.ids.insert(part.to_string(), ids.clone());
        ids
    }

    /// Get (loading if needed) the texture for one part id.
    fn texture(&mut self, ctx: &egui::Context, part: &str, id: i32) -> Option<TextureHandle> {
        let key = (part.to_string(), id);
        if let Some(t) = self.cache.get(&key) {
            return t.clone();
        }
        let path = self.dir.join(part).join(format!("{id}.png"));
        let handle = load_png(ctx, &path);
        self.cache.insert(key, handle.clone());
        handle
    }
}

fn load_png(ctx: &egui::Context, path: &std::path::Path) -> Option<TextureHandle> {
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
    Some(ctx.load_texture(path.to_string_lossy(), color, egui::TextureOptions::LINEAR))
}

/// A "None" option for parts that can be absent (mole, freckles).
pub fn none_button(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    ui.add_sized([72.0, 72.0], egui::Button::new(label).selected(selected))
}

/// A selectable thumbnail button at `px` size. Highlights when `selected`.
pub fn thumb_button(
    ui: &mut egui::Ui,
    cache: &mut ThumbCache,
    part: &str,
    id: i32,
    selected: bool,
    px: f32,
) -> egui::Response {
    let size = egui::vec2(px, px);
    let resp = match cache.texture(ui.ctx(), part, id) {
        Some(tex) => {
            let img = egui::Image::new(&tex).fit_to_exact_size(size);
            ui.add(egui::Button::image(img).selected(selected))
        }
        None => ui.add_sized(size, egui::Button::new(format!("{id}")).selected(selected)),
    };
    if selected {
        ui.painter().rect_stroke(
            resp.rect,
            5.0,
            egui::Stroke::new(2.0, aml_ui::theme::BLUE),
            egui::StrokeKind::Inside,
        );
    }
    resp.on_hover_text(format!("#{id}"))
}

