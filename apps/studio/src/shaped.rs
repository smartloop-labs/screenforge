//! Text egui cannot draw right on its own: scripts whose letters join or combine (Hindi,
//! Arabic, Bengali, Thai …). egui places one glyph per character, so these are shaped and
//! drawn by sf_render (rustybuzz) and shown as an image; other text is an ordinary label.

use egui::{Color32, RichText};

/// A wrapped label of `text` at `size` in `color`, shaped when its script needs it.
pub fn label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    if !sf_render::text::needs_shaping(text) {
        ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap());
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let width = ui.available_width().max(40.0);
    let key = egui::Id::new(("shaped_text", text, (width * ppp) as u32, (size * ppp * 10.0) as u32, color));
    let tex = match ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        Some(t) => t,
        None => {
            let Some(img) = rasterize(text, size * ppp, width * ppp, color) else { return };
            let image = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.rgba);
            let t = ui.ctx().load_texture("shaped_text", image, egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(key, t.clone()));
            t
        }
    };
    let size = tex.size_vec2() / ppp;
    // right-to-left text sits at the right
    let layout = if sf_render::text::line_is_rtl([text]) { egui::Layout::right_to_left(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) };
    ui.allocate_ui_with_layout(egui::vec2(width, size.y), layout, |ui| {
        ui.add(egui::Image::new((tex.id(), size))).on_hover_text(text);
    });
}

/// `text` wrapped at `width` px, its lines kept, in the interface's sans-serif at `size` px.
fn rasterize(text: &str, size: f32, width: f32, color: Color32) -> Option<sf_render::text::TextImage> {
    use sf_render::text::{rasterize_title, Align, FontFamily, TextImage, TitleText, TitleWord};
    let words: Vec<TitleWord> = text.lines().enumerate().flat_map(|(line, l)| l.split_whitespace().map(move |w| TitleWord { text: w.to_string(), accent: false, line, alpha: 1.0, blur_px: 0.0 })).collect();
    let [r, g, b, a] = color.to_srgba_unmultiplied().map(|v| v as f32 / 255.0);
    let c = sf_render::Color { r, g, b, a };
    let rtl = sf_render::text::line_is_rtl([text]);
    let t = TitleText { words, subtitle: vec![], family: FontFamily::Sans, size_px: size, color: c, accent: c, subtitle_color: c, subtitle_alpha: 0.0, align: if rtl { Align::Right } else { Align::Left }, shadow: false, max_width_px: width, pad_px: 1.0 };
    rasterize_title(&t).map(|i: TextImage| i)
}
