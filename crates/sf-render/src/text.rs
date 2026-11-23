//! Caption text rasterised on the CPU into a premultiplied RGBA image, then drawn as a
//! sprite. One line per image: a rounded background box with the words on it, or the words
//! alone over a soft shadow.
//!
//! Text is shaped with rustybuzz (so Devanagari conjuncts and Arabic joining come out right)
//! and drawn with ab_glyph. The bundled fonts cover Latin and Cyrillic; other scripts (Hindi,
//! Arabic, Chinese, Japanese, Korean …) are drawn with a system font that has them.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font, FontRef, Glyph, GlyphId, PxScale, ScaleFont};

use crate::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontFamily {
    Sans,
    Serif,
    Mono,
    /// Montserrat ExtraBold, for the highlight style.
    Heavy,
}

impl FontFamily {
    /// From the `captions.font` config value (`sans-serif` | `serif` | `mono` | `heavy`).
    pub fn from_config(s: &str) -> Self {
        match s {
            "serif" => FontFamily::Serif,
            "heavy" => FontFamily::Heavy,
            "mono" | "monospace" => FontFamily::Mono,
            _ => FontFamily::Sans,
        }
    }

    /// The bundled subsets, tried in order for each glyph (loaded once).
    fn fonts(self) -> &'static [Face] {
        static FACES: [OnceLock<Vec<Face>>; 4] = [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
        FACES[self as usize].get_or_init(|| self.load())
    }

    fn load(self) -> Vec<Face> {
        let data: [&'static [u8]; 3] = match self {
            FontFamily::Sans => [
                include_bytes!("../fonts/Inter-SemiBold-latin.ttf"),
                include_bytes!("../fonts/Inter-SemiBold-latin-ext.ttf"),
                include_bytes!("../fonts/Inter-SemiBold-cyrillic.ttf"),
            ],
            FontFamily::Serif => [
                include_bytes!("../fonts/SourceSerif4-SemiBold-latin.ttf"),
                include_bytes!("../fonts/SourceSerif4-SemiBold-latin-ext.ttf"),
                include_bytes!("../fonts/SourceSerif4-SemiBold-cyrillic.ttf"),
            ],
            FontFamily::Mono => [
                include_bytes!("../fonts/JetBrainsMono-SemiBold-latin.ttf"),
                include_bytes!("../fonts/JetBrainsMono-SemiBold-latin-ext.ttf"),
                include_bytes!("../fonts/JetBrainsMono-SemiBold-cyrillic.ttf"),
            ],
            FontFamily::Heavy => [
                include_bytes!("../fonts/Montserrat-ExtraBold-latin.ttf"),
                include_bytes!("../fonts/Montserrat-ExtraBold-latin-ext.ttf"),
                include_bytes!("../fonts/Montserrat-ExtraBold-cyrillic.ttf"),
            ],
        };
        // key symbols (⌘ ⇧ ⌥ ⏎ …) for every family: only Inter carries them
        let symbols: &'static [u8] = include_bytes!("../fonts/Inter-SemiBold-symbols.ttf");
        data.iter().chain([&symbols]).filter_map(|d| Face::new(d, 0)).collect()
    }
}

/// A font face: rustybuzz shapes text with it, ab_glyph draws its outlines.
pub struct Face {
    font: FontRef<'static>,
    hb: rustybuzz::Face<'static>,
    data: &'static [u8],
    index: u32,
}

impl Face {
    fn new(data: &'static [u8], index: u32) -> Option<Face> {
        Some(Face { font: FontRef::try_from_slice_and_index(data, index).ok()?, hb: rustybuzz::Face::from_slice(data, index)?, data, index })
    }

    fn covers(&self, c: char) -> bool {
        self.font.glyph_id(c).0 != 0
    }
}

/// System fonts tried first for scripts the bundled fonts lack (macOS's, then common Linux
/// ones), each at a bold weight when it has one.
const FALLBACK_FAMILIES: &[&str] = &[
    "Kohinoor Devanagari",
    "Noto Sans Devanagari",
    "Devanagari Sangam MN",
    "Geeza Pro",
    "Noto Sans Arabic",
    "PingFang SC",
    "Hiragino Sans",
    "Apple SD Gothic Neo",
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "Noto Sans CJK KR",
    "Noto Serif CJK SC",
    "Noto Serif CJK JP",
    "Noto Serif CJK KR",
    "Arial Unicode MS",
];

/// The installed fonts (read once).
fn system_fonts() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// System face `id`, mapped into memory once and kept for the run.
fn system_face(id: fontdb::ID) -> Option<&'static Face> {
    static LOADED: OnceLock<Mutex<HashMap<fontdb::ID, Option<&'static Face>>>> = OnceLock::new();
    static FILES: OnceLock<Mutex<HashMap<std::path::PathBuf, &'static [u8]>>> = OnceLock::new();
    let loaded = LOADED.get_or_init(Default::default);
    if let Some(f) = loaded.lock().ok()?.get(&id) {
        return *f;
    }
    let (source, index) = system_fonts().face_source(id)?;
    let data: Option<&'static [u8]> = match source {
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
            let mut files = FILES.get_or_init(Default::default).lock().ok()?;
            match files.get(&path) {
                Some(d) => Some(*d),
                None => {
                    let file = std::fs::File::open(&path).ok()?;
                    // SAFETY: system font files are not changed while the app runs; a mapping
                    // keeps a large CJK font out of memory until its glyphs are drawn
                    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
                    let d: &'static [u8] = Box::leak(Box::new(map));
                    files.insert(path, d);
                    Some(d)
                }
            }
        }
        fontdb::Source::Binary(b) => Some(Box::leak((*b).as_ref().to_vec().into_boxed_slice())),
    };
    let face = data.and_then(|d| Face::new(d, index)).map(|f| &*Box::leak(Box::new(f)));
    loaded.lock().ok()?.insert(id, face);
    face
}

/// A system face with a glyph for `c`: the first of [`FALLBACK_FAMILIES`] that has it, else
/// any installed font. Cached per character (`None`: no font has it).
fn fallback(c: char) -> Option<&'static Face> {
    static BY_CHAR: OnceLock<Mutex<HashMap<char, Option<&'static Face>>>> = OnceLock::new();
    let cache = BY_CHAR.get_or_init(Default::default);
    if let Some(f) = cache.lock().ok()?.get(&c) {
        return *f;
    }
    let db = system_fonts();
    let preferred = FALLBACK_FAMILIES.iter().filter_map(|name| {
        let q = fontdb::Query { families: &[fontdb::Family::Name(name)], weight: fontdb::Weight::BOLD, ..Default::default() };
        db.query(&q)
    });
    let found = preferred.chain(db.faces().map(|f| f.id)).filter_map(system_face).find(|f| f.covers(c));
    cache.lock().ok()?.insert(c, found);
    found
}

/// The system font file (its bytes and face index in a collection) that draws `c`, for
/// another renderer (the app's interface) to use as a fallback.
pub fn system_font_for(c: char) -> Option<(&'static [u8], u32)> {
    fallback(c).map(|f| (f.data, f.index))
}

/// `text` needs shaping to read right: letters that join or combine (Indic scripts, Arabic,
/// Hebrew, Thai …), which a renderer drawing one character at a time gets wrong.
pub fn needs_shaping(text: &str) -> bool {
    text.chars().any(|c| matches!(c as u32, 0x0590..=0x0DFF | 0x0E00..=0x0FFF | 0x1000..=0x109F | 0xFB1D..=0xFEFF))
}

/// Characters written right to left (Hebrew, Arabic and their presentation forms).
fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Characters written without spaces between words (Han, kana, CJK punctuation).
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFF60)
}

/// `text` is a right-to-left line (its first strong letter is).
pub fn line_is_rtl<'a>(words: impl IntoIterator<Item = &'a str>) -> bool {
    words.into_iter().flat_map(str::chars).find(|c| c.is_alphabetic()).is_some_and(is_rtl)
}

/// One word of the caption line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextWord {
    pub text: String,
    /// Takes part in the layout (false = not revealed yet).
    pub laid_out: bool,
    pub alpha: f32,
    /// Slide-in offset: 0 = settled, 1 = lowered by a third of the font size.
    pub drop01: f32,
    /// Drawn in [`CaptionText::highlight`] instead of [`CaptionText::color`].
    pub highlighted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionText {
    pub words: Vec<TextWord>,
    pub family: FontFamily,
    /// Font size in px.
    pub size_px: f32,
    pub color: Color,
    /// Colour of the highlighted word, or of its box with `highlight_box`.
    pub highlight: Color,
    /// The highlighted word keeps `color` and sits on a rounded box of `highlight`.
    pub highlight_box: bool,
    /// Box colour; a transparent one draws no box.
    pub background: Color,
    /// A soft dark shadow under the words, so they read without a box.
    pub shadow: bool,
    /// Only the part of the line up to the last revealed word gets a background, but the
    /// image keeps the full line's width (so a centred image grows to the right).
    pub box_to_revealed: bool,
    /// Widest the image may be; the font shrinks to fit.
    pub max_width_px: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8.
    pub rgba: Vec<u8>,
}

struct Shaper {
    fonts: &'static [Face],
    scale: PxScale,
}

/// A glyph placed by shaping: its face, id, and pen offset (px) from the word's start.
struct Placed {
    face: &'static Face,
    id: GlyphId,
    x: f32,
    y: f32,
}

impl Shaper {
    /// The face for `c`: a bundled one, else a system one, else the first bundled one.
    fn face_for(&self, c: char) -> &'static Face {
        self.fonts.iter().find(|f| f.covers(c)).or_else(|| fallback(c)).unwrap_or(&self.fonts[0])
    }

    /// `face` at the size of the bundled fonts' em (fonts differ in how tall their em is).
    fn scale_for(&self, face: &Face) -> PxScale {
        let main = &self.fonts[0].font;
        let em_px = self.scale.y * main.units_per_em().unwrap_or(1000.0) / main.height_unscaled().max(1.0);
        PxScale::from(em_px * face.font.height_unscaled() / face.font.units_per_em().unwrap_or(1000.0))
    }

    /// `text` shaped: its glyphs, and its advance (px). Runs of characters one face has are
    /// shaped together (a mark stays with its letter's face).
    fn shape(&self, text: &str) -> (Vec<Placed>, f32) {
        let mut runs: Vec<(&'static Face, String)> = vec![];
        for c in text.chars() {
            match runs.last_mut() {
                // zero-width joiners shape with the letters around them
                Some((f, run)) if f.covers(c) || matches!(c, '\u{200C}' | '\u{200D}') => run.push(c),
                _ => runs.push((self.face_for(c), c.to_string())),
            }
        }
        let mut out = vec![];
        let mut pen = 0.0;
        for (face, run) in runs {
            let mut buf = rustybuzz::UnicodeBuffer::new();
            buf.push_str(&run);
            buf.guess_segment_properties();
            let glyphs = rustybuzz::shape(&face.hb, &[], buf);
            let scaled = face.font.as_scaled(self.scale_for(face));
            let (kx, ky) = (scaled.h_scale_factor(), scaled.v_scale_factor());
            for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
                out.push(Placed { face, id: GlyphId(info.glyph_id as u16), x: pen + pos.x_offset as f32 * kx, y: -pos.y_offset as f32 * ky });
                pen += pos.x_advance as f32 * kx;
            }
        }
        (out, pen)
    }

    fn word_width(&self, text: &str) -> f32 {
        self.shape(text).1
    }

    fn space(&self) -> f32 {
        self.fonts[0].font.as_scaled(self.scale).h_advance(self.fonts[0].font.glyph_id(' '))
    }

    /// The gap between words `a` and `b`: a space, or none between words written without
    /// spaces (Chinese, Japanese).
    fn gap(&self, a: &str, b: &str) -> f32 {
        if a.chars().last().is_some_and(is_cjk) && b.chars().next().is_some_and(is_cjk) {
            0.0
        } else {
            self.space()
        }
    }

    fn ascent(&self) -> f32 {
        self.fonts[0].font.as_scaled(self.scale).ascent()
    }

    fn line_height(&self) -> f32 {
        let f = self.fonts[0].font.as_scaled(self.scale);
        f.ascent() - f.descent()
    }

    /// Draw `text` with its baseline at `(x, y)` into an alpha coverage buffer.
    fn draw(&self, text: &str, x: f32, y: f32, cov: &mut [f32], w: usize, h: usize) {
        for g in self.shape(text).0 {
            let glyph: Glyph = g.id.with_scale_and_position(self.scale_for(g.face), ab_glyph::point(x + g.x, y + g.y));
            if let Some(o) = g.face.font.outline_glyph(glyph) {
                let b = o.px_bounds();
                o.draw(|gx, gy, c| {
                    let px = b.min.x as i32 + gx as i32;
                    let py = b.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let i = py as usize * w + px as usize;
                        cov[i] = (cov[i] + c).min(1.0);
                    }
                });
            }
        }
    }
}

/// Signed distance to a rounded rect (negative inside).
fn rounded_rect_sd(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;
    let hx = (x1 - x0) / 2.0 - r;
    let hy = (y1 - y0) / 2.0 - r;
    let qx = (px - cx).abs() - hx;
    let qy = (py - cy).abs() - hy;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// Rasterise one caption line. `None` when nothing is visible.
pub fn rasterize_caption(c: &CaptionText) -> Option<TextImage> {
    let visible: Vec<&TextWord> = c.words.iter().filter(|w| w.laid_out).collect();
    if visible.is_empty() || c.size_px < 2.0 {
        return None;
    }
    let fonts = c.family.fonts();
    if fonts.is_empty() {
        return None;
    }
    // a right-to-left line (Arabic) runs from the right: its words are placed in reverse
    let rtl = line_is_rtl(visible.iter().map(|w| w.text.as_str()));
    let visible: Vec<&TextWord> = if rtl { visible.into_iter().rev().collect() } else { visible };
    let mut shaper = Shaper { fonts, scale: PxScale::from(c.size_px) };
    let measure = |s: &Shaper| -> (Vec<f32>, f32) {
        let widths: Vec<f32> = visible.iter().map(|w| s.word_width(&w.text)).collect();
        let gaps: f32 = visible.windows(2).map(|p| s.gap(&p[0].text, &p[1].text)).sum();
        (widths.clone(), widths.iter().sum::<f32>() + gaps)
    };
    let (mut widths, mut text_w) = measure(&shaper);
    let pad_x = c.size_px * 0.45;
    if text_w + 2.0 * pad_x > c.max_width_px && text_w > 0.0 {
        let k = ((c.max_width_px - 2.0 * pad_x) / text_w).max(0.2);
        shaper.scale = PxScale::from(c.size_px * k);
        (widths, text_w) = measure(&shaper);
    }
    let size = shaper.scale.x;
    let pad_x = size * 0.45;
    let pad_y = size * 0.28;
    let line_h = shaper.line_height();
    let w = (text_w + 2.0 * pad_x).ceil().max(1.0) as usize;
    let h = (line_h + 2.0 * pad_y + size * 0.35).ceil().max(1.0) as usize; // room for the slide
    let box_h = line_h + 2.0 * pad_y;

    // background box: up to the last revealed word, or the whole line (always, right to left)
    let box_to_revealed = c.box_to_revealed && !rtl;
    let mut x = pad_x;
    let mut box_right = pad_x;
    let mut placed = vec![];
    for (i, (word, ww)) in visible.iter().zip(&widths).enumerate() {
        if word.alpha > 0.0 || !box_to_revealed {
            box_right = x + ww;
        }
        placed.push((x, *word));
        x += ww + visible.get(i + 1).map_or(0.0, |next| shaper.gap(&word.text, &next.text));
    }
    let box_right = if box_to_revealed { box_right + pad_x } else { w as f32 };
    let radius = (box_h * 0.22).min(size * 0.35);
    let mut rgba = vec![0u8; w * h * 4];
    let bg = c.background;
    for py in (0..h).filter(|_| bg.a > 0.0) {
        for px in 0..w {
            let sd = rounded_rect_sd(px as f32 + 0.5, py as f32 + 0.5, 0.0, 0.0, box_right, box_h, radius);
            let a = (0.5 - sd).clamp(0.0, 1.0) * bg.a;
            if a > 0.0 {
                let i = (py * w + px) * 4;
                rgba[i] = (bg.r * a * 255.0).round() as u8;
                rgba[i + 1] = (bg.g * a * 255.0).round() as u8;
                rgba[i + 2] = (bg.b * a * 255.0).round() as u8;
                rgba[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }

    // words, each composited over the box with its own alpha
    let baseline = pad_y + shaper.ascent();
    let mut cov = vec![0f32; w * h];
    if c.shadow {
        // every visible word, spread and blurred, a little below the text
        let mut sh = vec![0f32; w * h];
        for (x, word) in &placed {
            if word.alpha > 0.0 {
                cov.iter_mut().for_each(|v| *v = 0.0);
                shaper.draw(&word.text, *x, baseline + word.drop01 * size * 0.33 + size * 0.04, &mut cov, w, h);
                sh.iter_mut().zip(&cov).for_each(|(s, c)| *s = s.max(c * word.alpha));
            }
        }
        let r = (size * 0.07).round().max(1.0) as usize;
        for _ in 0..2 {
            box_blur(&mut sh, w, h, r);
        }
        for (i, a) in sh.iter().enumerate() {
            let a = (a * 1.6).min(1.0) * 0.75;
            if a > 0.0 {
                let p = &mut rgba[i * 4..i * 4 + 4];
                p[3] = (a * 255.0 + p[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
                p[..3].iter_mut().for_each(|ch| *ch = (*ch as f32 * (1.0 - a)).round() as u8);
            }
        }
    }
    if c.highlight_box {
        for (x, word) in placed.iter().filter(|(_, w)| w.highlighted && w.alpha > 0.0) {
            // a little wider than the word, over the glyphs' full height
            let (pad, top) = (size * 0.16, baseline - shaper.ascent() + word.drop01 * size * 0.33);
            let (x0, x1) = (x - pad, x + shaper.word_width(&word.text) + pad);
            let (y0, y1) = (top - size * 0.02, top + line_h + size * 0.02);
            let hl = c.highlight;
            for py in (y0.floor().max(0.0) as usize)..(y1.ceil() as usize).min(h) {
                for px in (x0.floor().max(0.0) as usize)..(x1.ceil() as usize).min(w) {
                    let sd = rounded_rect_sd(px as f32 + 0.5, py as f32 + 0.5, x0, y0, x1, y1, size * 0.18);
                    let a = (0.5 - sd).clamp(0.0, 1.0) * word.alpha * hl.a;
                    if a > 0.0 {
                        let p = &mut rgba[(py * w + px) * 4..(py * w + px) * 4 + 4];
                        for (k, ch) in [hl.r, hl.g, hl.b].into_iter().enumerate() {
                            p[k] = (ch * a * 255.0 + p[k] as f32 * (1.0 - a)).round().min(255.0) as u8;
                        }
                        p[3] = (a * 255.0 + p[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
                    }
                }
            }
        }
    }
    for (x, word) in placed {
        if word.alpha <= 0.0 {
            continue;
        }
        cov.iter_mut().for_each(|v| *v = 0.0);
        shaper.draw(&word.text, x, baseline + word.drop01 * size * 0.33, &mut cov, w, h);
        let col = if word.highlighted && !c.highlight_box { c.highlight } else { c.color };
        for (i, cv) in cov.iter().enumerate() {
            let a = cv * word.alpha * col.a;
            if a <= 0.0 {
                continue;
            }
            let p = &mut rgba[i * 4..i * 4 + 4];
            for (k, ch) in [col.r, col.g, col.b].into_iter().enumerate() {
                p[k] = (ch * a * 255.0 + p[k] as f32 * (1.0 - a)).round().min(255.0) as u8;
            }
            p[3] = (a * 255.0 + p[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
        }
    }
    Some(TextImage { width: w as u32, height: h as u32, rgba })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn from_name(s: &str) -> Self {
        match s {
            "center" | "centre" => Align::Center,
            "right" => Align::Right,
            _ => Align::Left,
        }
    }
}

/// One word of a title.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleWord {
    pub text: String,
    /// Drawn in [`TitleText::accent`].
    pub accent: bool,
    /// Index of the written line it belongs to (long lines also wrap).
    pub line: usize,
    pub alpha: f32,
    /// Blur radius in px (0 = sharp).
    pub blur_px: f32,
}

/// A title: lines of words, some in the accent colour, and an optional smaller subtitle.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleText {
    pub words: Vec<TitleWord>,
    pub subtitle: Vec<String>,
    pub family: FontFamily,
    pub size_px: f32,
    pub color: Color,
    pub accent: Color,
    pub subtitle_color: Color,
    pub subtitle_alpha: f32,
    pub align: Align,
    pub shadow: bool,
    /// Lines wrap at this width.
    pub max_width_px: f32,
    /// Clear border around the text, room for blur and shadow.
    pub pad_px: f32,
}

/// Subtitle size and line spacing relative to the title size.
const SUBTITLE_SCALE: f32 = 0.42;
const TITLE_LEADING: f32 = 1.12;
const SUBTITLE_LEADING: f32 = 1.35;

/// Rasterise a title. The image is the text block plus `pad_px` on every side; lines are
/// aligned inside it by `align`. `None` when there is nothing to draw.
pub fn rasterize_title(t: &TitleText) -> Option<TextImage> {
    if (t.words.is_empty() && t.subtitle.is_empty()) || t.size_px < 2.0 {
        return None;
    }
    let fonts = t.family.fonts();
    if fonts.is_empty() {
        return None;
    }
    let title = Shaper { fonts, scale: PxScale::from(t.size_px) };
    let sub = Shaper { fonts, scale: PxScale::from(t.size_px * SUBTITLE_SCALE) };
    let max_w = t.max_width_px.max(t.size_px * 2.0);

    // greedy wrap: (word index, x) per visual line, plus each line's width
    let space = title.space();
    let mut lines: Vec<(Vec<(usize, f32)>, f32)> = vec![];
    let mut cur: Vec<(usize, f32)> = vec![];
    let mut x = 0.0f32;
    let mut written = t.words.first().map(|w| w.line).unwrap_or(0);
    for (i, w) in t.words.iter().enumerate() {
        let ww = title.word_width(&w.text);
        if !cur.is_empty() && (w.line != written || x + space + ww > max_w) {
            lines.push((std::mem::take(&mut cur), x));
            x = 0.0;
        }
        written = w.line;
        let at = if cur.is_empty() { 0.0 } else { x + space };
        cur.push((i, at));
        x = at + ww;
    }
    if !cur.is_empty() {
        lines.push((cur, x));
    }
    let sub_lines: Vec<(&str, f32)> = t.subtitle.iter().map(|l| (l.as_str(), sub.word_width(l))).collect();

    let title_lh = t.size_px * TITLE_LEADING;
    let sub_lh = t.size_px * SUBTITLE_SCALE * SUBTITLE_LEADING;
    let gap = if lines.is_empty() || sub_lines.is_empty() { 0.0 } else { t.size_px * 0.3 };
    let block_w = lines.iter().map(|l| l.1).chain(sub_lines.iter().map(|l| l.1)).fold(0.0f32, f32::max);
    let block_h = lines.len() as f32 * title_lh + gap + sub_lines.len() as f32 * sub_lh;
    let pad = t.pad_px.max(0.0);
    let (w, h) = ((block_w + 2.0 * pad).ceil().max(1.0) as usize, (block_h + 2.0 * pad).ceil().max(1.0) as usize);
    let line_x = |lw: f32| pad + match t.align {
        Align::Left => 0.0,
        Align::Center => (block_w - lw) / 2.0,
        Align::Right => block_w - lw,
    };

    let mut rgba = vec![0u8; w * h * 4];
    let mut shadow = t.shadow.then(|| vec![0f32; w * h]);
    // draw `text` at (x, baseline) into a small buffer, blur it, and composite it
    let mut put = |shaper: &Shaper, text: &str, x: f32, baseline: f32, col: Color, alpha: f32, blur: f32| {
        if alpha <= 0.0 {
            return;
        }
        let r = (blur * 0.5).round() as usize;
        let m = r * 3 + 2;
        let (x0, y0) = ((x.floor() as i64 - m as i64).max(0) as usize, ((baseline - shaper.ascent()).floor() as i64 - m as i64).max(0) as usize);
        let (bw, bh) = ((shaper.word_width(text).ceil() as usize + 2 * m + 2).min(w - x0.min(w)), (shaper.line_height().ceil() as usize + 2 * m + 2).min(h - y0.min(h)));
        if bw == 0 || bh == 0 {
            return;
        }
        let mut cov = vec![0f32; bw * bh];
        shaper.draw(text, x - x0 as f32, baseline - y0 as f32, &mut cov, bw, bh);
        if r > 0 {
            for _ in 0..3 {
                box_blur(&mut cov, bw, bh, r);
            }
        }
        for yy in 0..bh {
            for xx in 0..bw {
                let a = cov[yy * bw + xx] * alpha * col.a;
                if a <= 0.0 {
                    continue;
                }
                let i = (y0 + yy) * w + x0 + xx;
                if let Some(sh) = shadow.as_mut() {
                    sh[i] = sh[i].max(a);
                }
                let p = &mut rgba[i * 4..i * 4 + 4];
                for (k, ch) in [col.r, col.g, col.b].into_iter().enumerate() {
                    p[k] = (ch * a * 255.0 + p[k] as f32 * (1.0 - a)).round().min(255.0) as u8;
                }
                p[3] = (a * 255.0 + p[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
            }
        }
    };
    // right-to-left text (Arabic) runs from the right of each line
    let rtl = line_is_rtl(t.words.iter().map(|w| w.text.as_str()));
    let mut y = pad;
    for (line, lw) in &lines {
        let base = y + (title_lh - title.line_height()) / 2.0 + title.ascent();
        for (i, wx) in line {
            let word = &t.words[*i];
            let wx = if rtl { lw - wx - title.word_width(&word.text) } else { *wx };
            put(&title, &word.text, line_x(*lw) + wx, base, if word.accent { t.accent } else { t.color }, word.alpha, word.blur_px);
        }
        y += title_lh;
    }
    y += gap;
    for (text, lw) in &sub_lines {
        let base = y + (sub_lh - sub.line_height()) / 2.0 + sub.ascent();
        put(&sub, text, line_x(*lw), base, t.subtitle_color, t.subtitle_alpha, 0.0);
        y += sub_lh;
    }
    if let Some(mut sh) = shadow {
        // a soft shadow under everything drawn, composited behind the letters
        let r = (t.size_px * 0.06).round().max(1.0) as usize;
        for _ in 0..2 {
            box_blur(&mut sh, w, h, r);
        }
        for (i, a) in sh.iter().enumerate() {
            let a = (a * 1.4).min(1.0) * 0.6;
            let p = &mut rgba[i * 4..i * 4 + 4];
            let top = p[3] as f32 / 255.0;
            let under = a * (1.0 - top);
            p[3] = ((top + under) * 255.0).round().min(255.0) as u8;
        }
    }
    Some(TextImage { width: w as u32, height: h as u32, rgba })
}

/// Blur `buf` (`w`×`h`) with a `(2r+1)`-wide box, horizontally then vertically.
fn box_blur(buf: &mut [f32], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0f32; buf.len()];
    let k = 1.0 / (2 * r + 1) as f32;
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = buf[y * w + a..=y * w + b].iter().sum::<f32>() * k;
        }
    }
    for x in 0..w {
        for y in 0..h {
            let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
            buf[y * w + x] = (a..=b).map(|yy| tmp[yy * w + x]).sum::<f32>() * k;
        }
    }
}

/// A keyboard shortcut label: one key cap per key on a dark box.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyCaps {
    pub keys: Vec<String>,
    pub family: FontFamily,
    /// Height of a key cap in px.
    pub size_px: f32,
    pub color: Color,
    pub background: Color,
    pub max_width_px: f32,
}

/// Rasterise a shortcut label. `None` when there are no keys.
pub fn rasterize_keys(k: &KeyCaps) -> Option<TextImage> {
    if k.keys.is_empty() || k.size_px < 4.0 {
        return None;
    }
    let fonts = k.family.fonts();
    if fonts.is_empty() {
        return None;
    }
    // the glyphs take a bit over half the cap height
    let layout = |cap_h: f32| {
        let shaper = Shaper { fonts, scale: PxScale::from(cap_h * 0.52) };
        let pad = cap_h * 0.3;
        let gap = cap_h * 0.16;
        let caps: Vec<f32> = k.keys.iter().map(|t| (shaper.word_width(t) + 2.0 * pad).max(cap_h)).collect();
        let total = caps.iter().sum::<f32>() + gap * caps.len().saturating_sub(1) as f32 + 2.0 * gap;
        (shaper, caps, gap, total)
    };
    let mut cap_h = k.size_px;
    let (mut shaper, mut caps, mut gap, total) = layout(cap_h);
    if total > k.max_width_px {
        cap_h *= (k.max_width_px / total).max(0.2);
        (shaper, caps, gap, _) = layout(cap_h);
    }
    let w = (caps.iter().sum::<f32>() + gap * (caps.len() + 1) as f32).ceil() as usize;
    let h = (cap_h + 2.0 * gap).ceil() as usize;
    let mut rgba = vec![0u8; w * h * 4];
    let blend = |p: &mut [u8], c: Color, a: f32| {
        for (i, ch) in [c.r, c.g, c.b].into_iter().enumerate() {
            p[i] = (ch * a * 255.0 + p[i] as f32 * (1.0 - a)).round().min(255.0) as u8;
        }
        p[3] = (a * 255.0 + p[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
    };
    let outer_r = (gap + cap_h * 0.22).min(h as f32 / 2.0);
    let cap_r = cap_h * 0.22;
    let mut x0s = vec![];
    let mut x = gap;
    for cw in &caps {
        x0s.push(x);
        x += cw + gap;
    }
    for py in 0..h {
        for px in 0..w {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let p = &mut rgba[(py * w + px) * 4..(py * w + px) * 4 + 4];
            let a = (0.5 - rounded_rect_sd(fx, fy, 0.0, 0.0, w as f32, h as f32, outer_r)).clamp(0.0, 1.0) * k.background.a;
            if a > 0.0 {
                blend(p, k.background, a);
            }
            for (x0, cw) in x0s.iter().zip(&caps) {
                // a lighter cap with a darker lip along the bottom edge
                let sd = rounded_rect_sd(fx, fy, *x0, gap, x0 + cw, gap + cap_h, cap_r);
                let a = (0.5 - sd).clamp(0.0, 1.0);
                if a > 0.0 {
                    let lip = fy > gap + cap_h * 0.9;
                    blend(p, k.color, a * if lip { 0.08 } else { 0.16 });
                }
            }
        }
    }
    let line_h = shaper.line_height();
    let baseline = gap + (cap_h - line_h) / 2.0 + shaper.ascent() - cap_h * 0.03;
    let mut cov = vec![0f32; w * h];
    for ((t, x0), cw) in k.keys.iter().zip(&x0s).zip(&caps) {
        let tw = shaper.word_width(t);
        shaper.draw(t, x0 + (cw - tw) / 2.0, baseline, &mut cov, w, h);
    }
    for (i, cv) in cov.iter().enumerate() {
        let a = cv * k.color.a;
        if a > 0.0 {
            blend(&mut rgba[i * 4..i * 4 + 4], k.color, a);
        }
    }
    Some(TextImage { width: w as u32, height: h as u32, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caption(words: &[(&str, bool, f32)], box_to_revealed: bool) -> CaptionText {
        CaptionText {
            words: words.iter().map(|(t, l, a)| TextWord { text: t.to_string(), laid_out: *l, alpha: *a, drop01: 0.0, highlighted: false }).collect(),
            family: FontFamily::Sans,
            size_px: 40.0,
            color: Color::WHITE,
            highlight: Color::WHITE,
            highlight_box: false,
            background: Color::BLACK.with_alpha(0.75),
            shadow: false,
            box_to_revealed,
            max_width_px: 4000.0,
        }
    }

    fn ink(img: &TextImage) -> usize {
        // bright pixels = text over the dark box
        img.rgba.chunks_exact(4).filter(|p| p[0] > 128).count()
    }

    #[test]
    fn draws_text_on_a_box() {
        let img = rasterize_caption(&caption(&[("Hello", true, 1.0), ("world", true, 1.0)], false)).unwrap();
        assert!(img.width > 150 && img.height >= 40, "{}x{}", img.width, img.height);
        assert!(ink(&img) > 300, "text pixels: {}", ink(&img));
        // corners of the box are transparent (rounded), its middle row is opaque
        assert_eq!(img.rgba[3], 0);
        let mid = (img.height as usize / 2 * img.width as usize) * 4;
        assert!(img.rgba[mid + 3] > 150);
    }

    #[test]
    fn hidden_words_keep_space_but_draw_nothing() {
        let all = rasterize_caption(&caption(&[("Hello", true, 1.0), ("world", true, 1.0)], false)).unwrap();
        let half = rasterize_caption(&caption(&[("Hello", true, 1.0), ("world", true, 0.0)], false)).unwrap();
        assert_eq!(all.width, half.width);
        assert!(ink(&half) < ink(&all) * 2 / 3);
        assert!(rasterize_caption(&caption(&[("Hello", false, 0.0)], false)).is_none());
    }

    #[test]
    fn shrinks_to_the_width_limit_and_draws_cyrillic() {
        let mut c = caption(&[("Привет", true, 1.0), ("мир", true, 1.0)], true);
        c.max_width_px = 120.0;
        let img = rasterize_caption(&c).unwrap();
        assert!(img.width <= 121, "{}", img.width);
        assert!(ink(&img) > 20);
    }

    /// Pixels of `img` with text on them, for scripts the system may lack a font for: `None`
    /// when no installed font has `probe` (the test then has nothing to check).
    fn script_ink(words: &[&str], probe: char) -> Option<(TextImage, usize)> {
        fallback(probe)?;
        let c = caption(&words.iter().map(|w| (*w, true, 1.0)).collect::<Vec<_>>(), false);
        let img = rasterize_caption(&c).unwrap();
        let n = ink(&img);
        Some((img, n))
    }

    #[test]
    fn hindi_arabic_and_cjk_are_drawn_with_a_system_font() {
        for (words, probe) in [(&["नमस्ते", "दुनिया"][..], 'न'), (&["مرحبا", "بالعالم"][..], 'م'), (&["你好", "世界"][..], '你'), (&["こんにちは"][..], 'こ'), (&["안녕하세요"][..], '안')] {
            match script_ink(words, probe) {
                Some((_, n)) => assert!(n > 200, "{words:?}: {n} text pixels"),
                None => eprintln!("no installed font has {probe}; skipped"),
            }
        }
    }

    #[test]
    fn devanagari_is_shaped_into_conjuncts() {
        // क्ष is one conjunct glyph when shaped, three letters' worth of width when not
        let Some(face) = fallback('क') else { return };
        let s = Shaper { fonts: FontFamily::Sans.fonts(), scale: PxScale::from(40.0) };
        let (glyphs, _) = s.shape("क्ष");
        assert!(glyphs.len() < 3, "{} glyphs", glyphs.len());
        assert!(glyphs.iter().all(|g| std::ptr::eq(g.face, face)));
    }

    #[test]
    fn the_first_arabic_word_sits_on_the_right() {
        let red = |p: &[u8]| p[3] > 200 && p[0] > 180 && p[1] < 120;
        let halves = |img: &TextImage| {
            img.rgba.chunks_exact(4).enumerate().filter(|(_, p)| red(p)).fold((0, 0), |(l, r), (i, _)| if i % img.width as usize > img.width as usize / 2 { (l, r + 1) } else { (l + 1, r) })
        };
        if fallback('م').is_none() {
            return;
        }
        // a caption: the first (spoken) word highlighted
        let mut c = caption(&[("مرحبا", true, 1.0), ("بالعالم", true, 1.0), ("الجميل", true, 1.0)], false);
        c.highlight = Color::from_hex("#fa4a32").unwrap();
        c.words[0].highlighted = true;
        let (l, r) = halves(&rasterize_caption(&c).unwrap());
        assert!(r > 50 && l < r / 10, "caption: red left {l}, right {r}");
        // a wrapped block of text: the first word in the accent colour
        let mut t = title(&[("مرحبا", true, 0), ("بالعالم", false, 0), ("الجميل", false, 0)]);
        t.accent = Color::from_hex("#fa4a32").unwrap();
        t.align = Align::Right;
        let (l, r) = halves(&rasterize_title(&t).unwrap());
        assert!(r > 50 && l < r / 10, "title: red left {l}, right {r}");
    }

    #[test]
    fn scripts_that_need_shaping_are_told_apart() {
        assert!(needs_shaping("नमस्ते") && needs_shaping("مرحبا") && needs_shaping("Hi नमस्ते"));
        assert!(!needs_shaping("Hello, мир") && !needs_shaping("你好") && !needs_shaping("안녕"));
    }

    #[test]
    fn chinese_words_get_no_space_and_arabic_lines_run_right_to_left() {
        let s = Shaper { fonts: FontFamily::Sans.fonts(), scale: PxScale::from(40.0) };
        assert_eq!(s.gap("你好", "世界"), 0.0);
        assert!(s.gap("Hello", "world") > 0.0 && s.gap("你好", "world") > 0.0);
        assert!(line_is_rtl(["مرحبا", "بالعالم"]) && !line_is_rtl(["Hello", "مرحبا"]));
    }

    #[test]
    fn highlight_style_colours_the_spoken_word_over_a_shadow() {
        let mut c = caption(&[("Hello", true, 1.0), ("world", true, 1.0)], false);
        c.family = FontFamily::Heavy;
        c.background = Color::TRANSPARENT;
        c.shadow = true;
        c.highlight = Color::from_hex("#fa4a32").unwrap();
        c.words[1].highlighted = true;
        let img = rasterize_caption(&c).unwrap();
        let px: Vec<&[u8]> = img.rgba.chunks_exact(4).collect();
        let (left, right) = px.iter().enumerate().fold((0, 0), |(l, r), (i, p)| {
            // opaque reddish (premultiplied) pixels, by half of the image
            let red = p[3] > 200 && p[0] > 180 && p[1] < 120;
            if !red {
                (l, r)
            } else if i % img.width as usize > img.width as usize / 2 {
                (l, r + 1)
            } else {
                (l + 1, r)
            }
        });
        // "world" may start a hair left of the middle
        assert!(right > 100 && left < 20, "red pixels left {left}, right {right}");
        // no box: the corners stay clear, and the shadow darkens around the letters
        assert_eq!(img.rgba[3], 0);
        assert!(px.iter().any(|p| p[3] > 60 && p[0] < 40), "a dark shadow");
    }

    #[test]
    fn highlight_box_sits_behind_the_spoken_word() {
        let mut c = caption(&[("Hello", true, 1.0), ("world", true, 1.0)], false);
        c.background = Color::TRANSPARENT;
        c.highlight = Color::from_hex("#fa4a32").unwrap();
        c.highlight_box = true;
        c.words[1].highlighted = true;
        let img = rasterize_caption(&c).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let at = |x: usize, y: usize| &img.rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
        let red = |p: &[u8]| p[3] > 240 && p[0] > 200 && p[1] < 110;
        // an opaque red band across the second word, white letters on it, nothing behind the first
        let row = h * 2 / 5;
        assert!((w * 3 / 4..w * 3 / 4 + 4).any(|x| red(at(x, row))) || (0..h).any(|y| red(at(w * 3 / 4, y))));
        assert!((w / 2..w).flat_map(|x| (0..h).map(move |y| (x, y))).any(|(x, y)| at(x, y).iter().take(3).all(|v| *v > 230)), "white text on the box");
        assert!(!(0..w / 3).flat_map(|x| (0..h).map(move |y| (x, y))).any(|(x, y)| red(at(x, y))));
    }

    fn title(words: &[(&str, bool, usize)]) -> TitleText {
        TitleText {
            words: words.iter().map(|(t, a, l)| TitleWord { text: t.to_string(), accent: *a, line: *l, alpha: 1.0, blur_px: 0.0 }).collect(),
            subtitle: vec![],
            family: FontFamily::Sans,
            size_px: 48.0,
            color: Color::WHITE,
            accent: Color::from_hex("#8ea2ff").unwrap(),
            subtitle_color: Color::from_hex("#a1a1aa").unwrap(),
            subtitle_alpha: 1.0,
            align: Align::Left,
            shadow: false,
            max_width_px: 4000.0,
            pad_px: 10.0,
        }
    }

    #[test]
    fn titles_draw_lines_accents_and_a_subtitle() {
        let one = rasterize_title(&title(&[("Every", false, 0), ("move", true, 0)])).unwrap();
        let two = rasterize_title(&title(&[("Every", false, 0), ("move", true, 1)])).unwrap();
        assert!(two.height > one.height + 40 && two.width < one.width, "{}x{} vs {}x{}", one.width, one.height, two.width, two.height);
        // accent pixels are bluish: blue clearly above red
        let blue = one.rgba.chunks_exact(4).filter(|p| p[3] > 200 && p[2] as i32 - p[0] as i32 > 60).count();
        assert!(blue > 50, "accent pixels {blue}");
        let mut t = title(&[("Every", false, 0)]);
        t.subtitle = vec!["simulated, then predicted".into()];
        let with_sub = rasterize_title(&t).unwrap();
        assert!(with_sub.height > one.height);
        // a narrow width wraps the words
        let mut narrow = title(&[("Every", false, 0), ("move", false, 0), ("simulated", false, 0)]);
        narrow.max_width_px = 200.0;
        assert!(rasterize_title(&narrow).unwrap().height > one.height + 40);
    }

    #[test]
    fn hidden_and_blurred_words() {
        let sharp = rasterize_title(&title(&[("Hello", false, 0)])).unwrap();
        let mut t = title(&[("Hello", false, 0)]);
        t.words[0].blur_px = 8.0;
        let soft = rasterize_title(&t).unwrap();
        let solid = |img: &TextImage| img.rgba.chunks_exact(4).filter(|p| p[3] > 240).count();
        assert_eq!((soft.width, soft.height), (sharp.width, sharp.height), "blur does not move the layout");
        assert!(solid(&soft) * 3 < solid(&sharp), "{} vs {}", solid(&soft), solid(&sharp));
        t.words[0].alpha = 0.0;
        assert!(rasterize_title(&t).unwrap().rgba.chunks_exact(4).all(|p| p[3] == 0));
    }

    fn keys(k: &[&str]) -> KeyCaps {
        KeyCaps { keys: k.iter().map(|s| s.to_string()).collect(), family: FontFamily::Sans, size_px: 64.0, color: Color::WHITE, background: Color::BLACK.with_alpha(0.78), max_width_px: 4000.0 }
    }

    #[test]
    fn key_caps_draw_symbols_from_the_fallback_font() {
        let img = rasterize_keys(&keys(&["⇧", "⌘", "S"])).unwrap();
        // three caps at least as wide as they are tall
        assert!(img.width as f32 >= 3.0 * 64.0 && img.height >= 64, "{}x{}", img.width, img.height);
        // each symbol alone draws ink, so ⌘ and ⇧ are not missing glyphs
        for k in ["⌘", "⇧", "⌥", "⌃", "⏎", "⌫"] {
            let one = rasterize_keys(&keys(&[k])).unwrap();
            assert!(ink(&one) > 30, "{k}: {}", ink(&one));
        }
        assert!(rasterize_keys(&keys(&[])).is_none());
    }

    #[test]
    fn key_caps_shrink_to_the_width_limit() {
        let mut k = keys(&["⌃", "⌥", "⇧", "⌘", "Space"]);
        k.max_width_px = 200.0;
        assert!(rasterize_keys(&k).unwrap().width <= 201);
    }
}
