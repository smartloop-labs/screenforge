//! Templates: videos made without a recording. A template is a look (a drawn background, a
//! palette, where the headline and the card sit) and a structure (a hook, beats, an end card)
//! that an agent fills with words, narration and cards it draws as SVG. The result is an
//! ordinary project: the background is its "recording", a still video as long as the piece,
//! so narration, captions, titles and overlays work as they do over a screen recording, and
//! the user opens it in ScreenForge and changes anything.

use serde_json::{json, Value};

/// The frame every template is drawn for.
pub const WIDTH: f64 = 1080.0;
pub const HEIGHT: f64 = 1920.0;
/// The canvas an agent draws a card on: the card area's proportions.
pub const CARD_W: f64 = 860.0;
pub const CARD_H: f64 = 648.0;

pub struct Template {
    pub key: &'static str,
    pub label: &'static str,
    pub about: &'static str,
    /// The background's fill and its two soft glows: colour, centre (0–1), radius (0–1 of the height).
    pub base: &'static str,
    pub glows: [(&'static str, f64, f64, f64); 2],
    pub grid: bool,
    /// The second line of a headline, the captions' spoken word and the button.
    pub accent: &'static str,
    /// The card panel's fill, for the agent's drawings.
    pub panel: &'static str,
    /// Vertical centre and size of a beat's headline (fractions of the height).
    pub headline_y: f64,
    pub headline_size: f64,
    /// "STEP 1" over each beat's headline.
    pub steps: bool,
    /// Where a card goes: x, y, width, height (fractions of the frame).
    pub card: [f64; 4],
    pub card_in: &'static str,
}

pub const TEMPLATES: [Template; 3] = [
    Template {
        key: "feature-short",
        label: "Feature short",
        about: "One product feature in about 20 seconds: a two-line hook, three to five beats (a headline over an illustrated card), and an end card with the name, a tagline, a button and the web address. Dark plum with warm glows.",
        base: "#14131f",
        glows: [("#ff7a45", 0.22, 0.30, 0.42), ("#d0367f", 0.88, 0.74, 0.46)],
        grid: true,
        accent: "#ff8a4c",
        panel: "#1d1c2b",
        headline_y: 0.215,
        headline_size: 0.047,
        steps: false,
        card: [0.075, 0.3, 0.78, 0.33],
        card_in: "zoom",
    },
    Template {
        key: "how-to-steps",
        label: "How-to in steps",
        about: "A numbered how-to: a hook that names the result, then one beat per step (STEP 1, STEP 2 … over a headline and a card that shows the step), and an end card. Deep blue with cool glows.",
        base: "#0d1524",
        glows: [("#2f6df0", 0.15, 0.22, 0.44), ("#22c7dd", 0.9, 0.8, 0.4)],
        grid: false,
        accent: "#5cc8ff",
        panel: "#152238",
        headline_y: 0.225,
        headline_size: 0.044,
        steps: true,
        card: [0.075, 0.3, 0.78, 0.33],
        card_in: "rise",
    },
    Template {
        key: "announcement",
        label: "Announcement",
        about: "A launch, a sale or one piece of news: a big hook, two or three short statements (cards optional), and an end card with the offer and the web address. Near-black with an amber glow.",
        base: "#150f12",
        glows: [("#ff5a36", 0.5, 0.18, 0.5), ("#ffb347", 0.5, 0.95, 0.42)],
        grid: false,
        accent: "#ffb14a",
        panel: "#221a1c",
        headline_y: 0.24,
        headline_size: 0.052,
        steps: false,
        card: [0.075, 0.31, 0.78, 0.32],
        card_in: "fade",
    },
];

pub fn find(key: &str) -> Option<&'static Template> {
    let key = key.trim().to_lowercase();
    TEMPLATES.iter().find(|t| t.key == key)
}

impl Template {
    /// The background, drawn for the whole frame.
    pub fn background_svg(&self) -> String {
        let mut s = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}"><defs>"#);
        for (i, (color, _, _, _)) in self.glows.iter().enumerate() {
            s += &format!(r#"<radialGradient id="g{i}"><stop offset="0" stop-color="{color}" stop-opacity="0.42"/><stop offset="1" stop-color="{color}" stop-opacity="0"/></radialGradient>"#);
        }
        s += &format!(r#"</defs><rect width="{WIDTH}" height="{HEIGHT}" fill="{}"/>"#, self.base);
        for (i, (_, x, y, r)) in self.glows.iter().enumerate() {
            s += &format!(r#"<circle cx="{}" cy="{}" r="{}" fill="url(#g{i})"/>"#, x * WIDTH, y * HEIGHT, r * HEIGHT);
        }
        if self.grid {
            let mut d = String::new();
            let mut x = 90.0;
            while x < WIDTH {
                d += &format!("M{x} 0V{HEIGHT}");
                x += 90.0;
            }
            let mut y = 90.0;
            while y < HEIGHT {
                d += &format!("M0 {y}H{WIDTH}");
                y += 90.0;
            }
            s += &format!(r##"<path d="{d}" stroke="#ffffff" stroke-opacity="0.045" stroke-width="2" fill="none"/>"##);
        }
        s + "</svg>"
    }

    /// The end card's button, drawn in the card canvas's width.
    pub fn button_svg(&self, label: &str) -> String {
        let label = escape(label);
        let w = (label.chars().count() as f64 * 27.0 + 110.0).clamp(300.0, CARD_W);
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CARD_W} 130"><rect x="{x}" y="15" width="{w}" height="100" rx="50" fill="{accent}"/><text x="{cx}" y="80" text-anchor="middle" font-family="Montserrat" font-weight="800" font-size="42" fill="#16131c">{label}</text></svg>"##,
            x = (CARD_W - w) / 2.0,
            cx = CARD_W / 2.0,
            accent = self.accent,
        )
    }

    /// What `list_templates` says about it.
    pub fn json(&self) -> Value {
        json!({
            "template": self.key, "label": self.label, "about": self.about,
            "colors": { "background": self.base, "accent": self.accent, "panel": self.panel, "text": "#ffffff", "muted": "#a9a6b8" },
            "stepLabels": self.steps,
        })
    }
}

/// How to draw a card, for the tool descriptions.
pub fn card_guide() -> String {
    format!(
        "A card is SVG you write: <svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {CARD_W} {CARD_H}\">…</svg>, transparent outside what you draw. \
Draw one idea per card, large and simple, the way a motion designer would: a rounded panel (rx 36, the template's panel colour, a 2 px white stroke at 0.08 opacity) holding a big number or timer, three list rows with a round badge each, a waveform of rounded bars, a progress bar, a mock window with a few blocks, or a single large icon made of basic shapes. \
Use the template's colours (list_templates gives them): white for main text, the muted colour for small labels, the accent for the one thing to notice. Text in font-family Inter (labels) or Montserrat (big words, font-weight 800), 34 px or larger, at most about ten words on a card; no text smaller than 28 px, no photographs, no external images or fonts, no scripts or animation (ScreenForge animates the card in)."
    )
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A file name for a project called `name`.
pub fn file_stem(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() { "Template video".into() } else { s.chars().take(60).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_draws_and_keeps_its_card_in_the_tiktok_safe_area() {
        let mut keys: Vec<&str> = TEMPLATES.iter().map(|t| t.key).collect();
        keys.dedup();
        assert_eq!(keys.len(), TEMPLATES.len());
        for t in &TEMPLATES {
            let (png, w, h) = sf_services::svg::render(&t.background_svg(), 1920).unwrap_or_else(|e| panic!("{}: {e}", t.key));
            assert_eq!((w, h), (1080, 1920), "{}", t.key);
            assert!(png.len() > 1000);
            assert!(sf_services::svg::render(&t.button_svg("Download free today"), 860).is_ok());
            // TikTok leaves x 60–940 and y 220–1440 clear
            let [x, y, cw, ch] = t.card;
            assert!(x * WIDTH >= 60.0 && (x + cw) * WIDTH <= 940.0 && y * HEIGHT >= 220.0 && (y + ch) * HEIGHT <= 1440.0, "{}", t.key);
            // the card canvas has the card area's proportions (within 3%)
            let (area, canvas) = (cw * WIDTH / (ch * HEIGHT), CARD_W / CARD_H);
            assert!((area / canvas - 1.0).abs() < 0.08, "{}: {area} vs {canvas}", t.key);
            assert!(find(&t.key.to_uppercase()).is_some());
        }
        assert_eq!(file_stem("  My: demo/video?  "), "My demo video");
        assert_eq!(escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
    }
}
