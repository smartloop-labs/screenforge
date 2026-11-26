//! Templates: videos made without a recording. A template is a look (a drawn background, a
//! palette, where the headline and the card sit) and a structure (a hook, beats, an end card)
//! that a person or an agent fills with words, narration and cards. The result is an ordinary
//! project: the background is its "recording", a still video as long as the piece, so
//! narration, captions, titles and overlays work as they do over a screen recording.
//!
//! A card is not one picture: it is laid out as separate elements ([`Element`]), each shape an
//! overlay of its own and each piece of text a title, so every one can be selected, moved,
//! retyped or deleted in the editor, like the layers of an ungrouped drawing.

use serde_json::{json, Value};

/// The frame every template is drawn for.
pub const WIDTH: f64 = 1080.0;
pub const HEIGHT: f64 = 1920.0;
/// The canvas a card is laid out on: the card area's proportions.
pub const CARD_W: f64 = 860.0;
pub const CARD_H: f64 = 648.0;
/// The colour of main text, of small labels, and of text on the accent colour.
pub const TEXT: &str = "#ffffff";
pub const MUTED: &str = "#a9a6b8";
pub const INK: &str = "#16131c";

/// What is drawn over the background's glows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pattern {
    None,
    Grid,
    Dots,
    Lines,
}

/// The small label over each beat's headline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Labels {
    None,
    /// "STEP 1", "STEP 2" …
    Numbered(&'static str),
    /// One per beat, in order; beats past the last have none.
    Fixed(&'static [&'static str]),
}

pub struct Template {
    pub key: &'static str,
    pub label: &'static str,
    pub about: &'static str,
    /// The background's fill and its two soft glows: colour, centre (0–1), radius (0–1 of the height).
    pub base: &'static str,
    pub glows: [(&'static str, f64, f64, f64); 2],
    pub pattern: Pattern,
    /// The second line of a headline, the captions' spoken word and the button.
    pub accent: &'static str,
    /// The card panel's fill.
    pub panel: &'static str,
    /// Vertical centre and size of a beat's headline (fractions of the height).
    pub headline_y: f64,
    pub headline_size: f64,
    pub labels: Labels,
    /// How a card's elements come in.
    pub card_in: &'static str,
}

/// Where a card goes in every template: x, y, width, height (fractions of the frame), inside
/// the safe area of TikTok, Reels and Shorts and in the proportions of the card canvas.
pub const CARD: [f64; 4] = [0.075, 0.3, 0.78, 0.3306];

const fn template(key: &'static str, label: &'static str, about: &'static str, base: &'static str, glows: [(&'static str, f64, f64, f64); 2], pattern: Pattern, accent: &'static str, panel: &'static str, labels: Labels, card_in: &'static str) -> Template {
    Template { key, label, about, base, glows, pattern, accent, panel, headline_y: 0.22, headline_size: 0.046, labels, card_in }
}

pub const TEMPLATES: [Template; 9] = [
    template("feature-short", "Feature short", "One product feature in about 20 seconds: a two-line hook, three to five beats (a headline over a card), and an end card with the name, a tagline, a button and the web address. Dark plum with warm glows.", "#14131f", [("#ff7a45", 0.22, 0.30, 0.42), ("#d0367f", 0.88, 0.74, 0.46)], Pattern::Grid, "#ff8a4c", "#1d1c2b", Labels::None, "zoom"),
    template("how-to-steps", "How-to in steps", "A numbered how-to: a hook that names the result, then one beat per step (STEP 1, STEP 2 … over a headline and a card that shows the step), and an end card. Deep blue with cool glows.", "#0d1524", [("#2f6df0", 0.15, 0.22, 0.44), ("#22c7dd", 0.9, 0.8, 0.4)], Pattern::None, "#5cc8ff", "#152238", Labels::Numbered("STEP"), "rise"),
    template("announcement", "Announcement", "A launch or one piece of news: a big hook, two or three short statements with a card each, and an end card with the web address. Near-black with an amber glow.", "#150f12", [("#ff5a36", 0.5, 0.18, 0.5), ("#ffb347", 0.5, 0.95, 0.42)], Pattern::None, "#ffb14a", "#221a1c", Labels::None, "fade"),
    template("top-three", "Top three", "A countdown list: a hook that names the list, then number three, two and one, each a headline over a card, and an end card. Charcoal with gold.", "#101418", [("#f59e0b", 0.2, 0.25, 0.4), ("#ef4444", 0.86, 0.8, 0.42)], Pattern::Dots, "#fbbf24", "#1b2027", Labels::Fixed(&["NUMBER 3", "NUMBER 2", "NUMBER 1"]), "zoom"),
    template("quick-tip", "Quick tip", "One tip in fifteen seconds: the problem, the fix, and what it gives you. Dark green with fresh glows.", "#0c1a17", [("#10b981", 0.2, 0.2, 0.45), ("#84cc16", 0.86, 0.85, 0.4)], Pattern::Lines, "#34d399", "#132621", Labels::Fixed(&["THE PROBLEM", "THE FIX", "THE RESULT"]), "rise"),
    template("did-you-know", "Did you know", "A surprising fact: the fact as a big figure, why it is so, and what to do with it. Deep violet with pink.", "#170f24", [("#8b5cf6", 0.25, 0.25, 0.45), ("#ec4899", 0.86, 0.8, 0.42)], Pattern::Dots, "#c4b5fd", "#22183a", Labels::None, "zoom"),
    template("before-after", "Before and after", "A comparison: how it was, how it is now, and the difference in one figure. Graphite, from red to green.", "#121316", [("#ef4444", 0.12, 0.3, 0.4), ("#22c55e", 0.9, 0.7, 0.42)], Pattern::None, "#4ade80", "#1c1e24", Labels::Fixed(&["BEFORE", "AFTER", "THE DIFFERENCE"]), "fade"),
    template("testimonial", "Testimonial", "What a customer said: their words as a quote, the result they got as a figure, and an end card. Warm brown with orange.", "#17120e", [("#f97316", 0.2, 0.2, 0.4), ("#eab308", 0.86, 0.85, 0.38)], Pattern::None, "#fdba74", "#241c16", Labels::None, "fade"),
    template("sale", "Sale", "An offer: the price and what it was, what you get, and when it ends, closing on a button. Dark red with coral.", "#1a0b10", [("#f43f5e", 0.5, 0.2, 0.5), ("#fb923c", 0.5, 0.92, 0.4)], Pattern::Lines, "#ff6b81", "#2a1219", Labels::None, "zoom"),
];

pub fn find(key: &str) -> Option<&'static Template> {
    let key = key.trim().to_lowercase();
    TEMPLATES.iter().find(|t| t.key == key)
}

/// One piece of a card, in the card canvas's pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// Artwork without words (a panel, a row, a badge, a bar): its own image.
    Shape { svg: String, x: f64, y: f64, w: f64, h: f64 },
    /// Words: a title in the editor. `x` is the left edge, centre or right edge by `align`;
    /// `y` the vertical centre; `size` the font size.
    Text { text: String, x: f64, y: f64, size: f64, align: &'static str, color: String, heavy: bool },
}

fn shape(x: f64, y: f64, w: f64, h: f64, inner: String) -> Element {
    Element::Shape { svg: format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}">{inner}</svg>"#), x, y, w, h }
}

fn words(text: &str, x: f64, y: f64, size: f64, align: &'static str, color: &str, heavy: bool) -> Element {
    Element::Text { text: text.trim().to_string(), x, y, size, align, color: color.to_string(), heavy }
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
        let mut d = String::new();
        match self.pattern {
            Pattern::None => {}
            Pattern::Grid => {
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
            }
            Pattern::Lines => {
                // diagonals, bottom-left to top-right
                let mut c = -HEIGHT;
                while c < WIDTH {
                    d += &format!("M{c} {HEIGHT}L{} 0", c + HEIGHT);
                    c += 120.0;
                }
            }
            Pattern::Dots => {
                let mut y = 60.0;
                while y < HEIGHT {
                    let mut x = 60.0;
                    while x < WIDTH {
                        s += &format!(r##"<circle cx="{x}" cy="{y}" r="3" fill="#ffffff" fill-opacity="0.07"/>"##);
                        x += 120.0;
                    }
                    y += 120.0;
                }
            }
        }
        if !d.is_empty() {
            s += &format!(r##"<path d="{d}" stroke="#ffffff" stroke-opacity="0.045" stroke-width="2" fill="none"/>"##);
        }
        s + "</svg>"
    }

    /// The label over beat `index`'s headline (0-based), when the template has one.
    pub fn beat_label(&self, index: usize) -> Option<String> {
        match self.labels {
            Labels::None => None,
            Labels::Numbered(word) => Some(format!("{word} {}", index + 1)),
            Labels::Fixed(list) => list.get(index).map(|s| s.to_string()),
        }
    }

    fn panel(&self) -> Element {
        shape(20.0, 20.0, CARD_W - 40.0, CARD_H - 40.0, format!(r##"<rect x="1" y="1" width="{}" height="{}" rx="36" fill="{}" stroke="#ffffff" stroke-opacity="0.08" stroke-width="2"/>"##, CARD_W - 42.0, CARD_H - 42.0, self.panel))
    }

    /// A card as its separate elements. `card` is `{ type, … }`:
    /// `list` / `checklist` { label, rows }, `stat` { label, value, caption }, `quote` { text, by },
    /// `compare` { left: { label, value }, right: { label, value } }, `price` { was, value, caption },
    /// `window` { label }.
    pub fn card_elements(&self, card: &Value) -> Result<Vec<Element>, String> {
        let get = |v: &Value, k: &str| v[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let kind = get(card, "type").ok_or("a card needs a type: list, checklist, stat, quote, compare, price or window (or give SVG markup)")?;
        let mut out = vec![self.panel()];
        let label = get(card, "label");
        if let Some(l) = &label {
            out.push(words(l, 70.0, 92.0, 34.0, "left", MUTED, false));
        }
        let top = if label.is_some() { 140.0 } else { 70.0 };
        let cx = CARD_W / 2.0;
        match kind.as_str() {
            "list" | "checklist" => {
                let rows: Vec<String> = card["rows"].as_array().into_iter().flatten().filter_map(|r| r.as_str()).map(str::trim).filter(|r| !r.is_empty()).map(str::to_string).take(4).collect();
                if rows.is_empty() {
                    return Err("a list card needs rows: one to four short lines".into());
                }
                let gap = 26.0;
                let rh = ((CARD_H - top - 60.0 - gap * (rows.len() as f64 - 1.0)) / rows.len() as f64).min(120.0);
                for (i, row) in rows.iter().enumerate() {
                    let y = top + i as f64 * (rh + gap);
                    let mark = if kind == "checklist" {
                        format!(r#"<path d="M48 {m} l12 13 l24 -27" stroke="{INK}" stroke-width="8" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#, m = rh / 2.0 + 1.0)
                    } else {
                        format!(r#"<text x="65" y="{}" text-anchor="middle" font-family="Montserrat" font-weight="800" font-size="34" fill="{INK}">{}</text>"#, rh / 2.0 + 12.0, i + 1)
                    };
                    out.push(shape(60.0, y, 740.0, rh, format!(r##"<rect width="740" height="{rh}" rx="26" fill="#ffffff" fill-opacity="0.05"/><circle cx="65" cy="{}" r="34" fill="{}"/>{mark}"##, rh / 2.0, self.accent)));
                    out.push(words(row, 185.0, y + rh / 2.0, 40.0, "left", TEXT, true));
                }
            }
            "stat" => {
                out.push(words(&get(card, "value").ok_or("a stat card needs a value: a figure or a word or two")?, cx, top + 150.0, 170.0, "center", TEXT, true));
                out.push(shape(130.0, top + 300.0, 600.0, 28.0, format!(r##"<rect width="600" height="28" rx="14" fill="#ffffff" fill-opacity="0.08"/><rect width="420" height="28" rx="14" fill="{}"/>"##, self.accent)));
                if let Some(c) = get(card, "caption") {
                    out.push(words(&c, cx, top + 390.0, 36.0, "center", MUTED, false));
                }
            }
            "quote" => {
                out.push(shape(60.0, top - 10.0, 120.0, 100.0, format!(r#"<text x="0" y="110" font-family="Montserrat" font-weight="800" font-size="170" fill="{}">“</text>"#, self.accent)));
                out.push(words(&get(card, "text").ok_or("a quote card needs text: what was said")?, cx, top + 210.0, 46.0, "center", TEXT, true));
                if let Some(by) = get(card, "by") {
                    out.push(words(&by, cx, CARD_H - 90.0, 34.0, "center", MUTED, false));
                }
            }
            "compare" => {
                for (i, side) in ["left", "right"].iter().enumerate() {
                    let x = 60.0 + i as f64 * 380.0;
                    let fill = if i == 1 { format!(r#"fill="{}" fill-opacity="0.16" stroke="{}" stroke-width="3""#, self.accent, self.accent) } else { r##"fill="#ffffff" fill-opacity="0.05""##.to_string() };
                    out.push(shape(x, top, 360.0, CARD_H - top - 60.0, format!(r#"<rect x="2" y="2" width="356" height="{}" rx="26" {fill}/>"#, CARD_H - top - 64.0)));
                    if let Some(l) = get(&card[*side], "label") {
                        out.push(words(&l, x + 180.0, top + 70.0, 34.0, "center", MUTED, false));
                    }
                    out.push(words(&get(&card[*side], "value").ok_or("a compare card needs left and right, each { label, value }")?, x + 180.0, top + (CARD_H - top - 60.0) / 2.0 + 30.0, 84.0, "center", if i == 1 { self.accent } else { TEXT }, true));
                }
            }
            "price" => {
                if let Some(was) = get(card, "was") {
                    out.push(words(&was, cx, top + 50.0, 48.0, "center", MUTED, false));
                    out.push(shape(cx - 90.0, top + 50.0, 180.0, 8.0, format!(r#"<rect width="180" height="6" rx="3" fill="{}"/>"#, self.accent)));
                }
                out.push(words(&get(card, "value").ok_or("a price card needs a value: the price")?, cx, top + 210.0, 180.0, "center", TEXT, true));
                if let Some(c) = get(card, "caption") {
                    out.push(shape(cx - 230.0, top + 345.0, 460.0, 80.0, format!(r#"<rect width="460" height="80" rx="40" fill="{}"/>"#, self.accent)));
                    out.push(words(&c, cx, top + 385.0, 36.0, "center", INK, true));
                }
            }
            "window" => {
                let a = self.accent;
                out.push(shape(60.0, top, 740.0, CARD_H - top - 60.0, format!(r##"<rect width="740" height="{h}" rx="26" fill="#ffffff" fill-opacity="0.05"/><circle cx="40" cy="40" r="10" fill="{a}"/><circle cx="72" cy="40" r="10" fill="#ffffff" fill-opacity="0.2"/><circle cx="104" cy="40" r="10" fill="#ffffff" fill-opacity="0.2"/><rect x="30" y="80" width="220" height="{b}" rx="16" fill="#ffffff" fill-opacity="0.06"/><rect x="280" y="80" width="430" height="{c}" rx="16" fill="{a}" fill-opacity="0.85"/><rect x="280" y="{d}" width="205" height="{c}" rx="16" fill="#ffffff" fill-opacity="0.06"/><rect x="505" y="{d}" width="205" height="{c}" rx="16" fill="#ffffff" fill-opacity="0.06"/>"##, h = CARD_H - top - 60.0, b = CARD_H - top - 170.0, c = (CARD_H - top - 195.0) / 2.0, d = 105.0 + (CARD_H - top - 195.0) / 2.0)));
            }
            other => return Err(format!("unknown card type {other}: list, checklist, stat, quote, compare, price or window (or give SVG markup)")),
        }
        Ok(out)
    }

    /// The end card's button as elements on a strip `CARD_W` wide and 130 high.
    pub fn button_elements(&self, label: &str) -> Vec<Element> {
        let w = (label.chars().count() as f64 * 27.0 + 110.0).clamp(300.0, CARD_W);
        vec![shape((CARD_W - w) / 2.0, 15.0, w, 100.0, format!(r#"<rect width="{w}" height="100" rx="50" fill="{}"/>"#, self.accent)), words(label, CARD_W / 2.0, 65.0, 42.0, "center", INK, true)]
    }

    /// What `list_templates` says about it.
    pub fn json(&self) -> Value {
        json!({
            "template": self.key, "label": self.label, "about": self.about,
            "colors": { "background": self.base, "accent": self.accent, "panel": self.panel, "text": TEXT, "muted": MUTED },
            "beatLabels": match self.labels { Labels::None => json!(null), Labels::Numbered(w) => json!(format!("{w} 1, {w} 2 …")), Labels::Fixed(l) => json!(l) },
        })
    }

    /// Stand-in words and cards for a project started from the template by hand, in the shape
    /// `new_from_template` takes: every title is there to be typed over.
    pub fn sample(&self) -> Value {
        let list = |label: &str| json!({ "type": "list", "label": label, "rows": ["First point", "Second point", "Third point"] });
        let check = |label: &str| json!({ "type": "checklist", "label": label, "rows": ["One thing it does", "Another thing", "And one more"] });
        let stat = |label: &str, value: &str, caption: &str| json!({ "type": "stat", "label": label, "value": value, "caption": caption });
        let window = json!({ "type": "window", "label": "How it looks" });
        let (hook, beats, end): ((&str, &str), Vec<(&str, &str, Value)>, (&str, &str)) = match self.key {
            "how-to-steps" => (("How to do it", "in three steps"), vec![("Start here", "the first step", list("What you need")), ("Then this", "the second step", window), ("Finish with", "the last step", check("You are done when"))], ("Your product", "Try it free")),
            "announcement" => (("Big news", "goes here"), vec![("What is new", "say it short", stat("New", "2.0", "What it is called")), ("Why it matters", "one reason", check("What you get"))], ("Your product", "See what is new")),
            "top-three" => (("Top three", "things to know"), vec![("Third place", "why it is here", stat("Number three", "3", "One line about it")), ("Second place", "why it is here", stat("Number two", "2", "One line about it")), ("First place", "the winner", stat("Number one", "1", "One line about it"))], ("Your channel", "Follow for more")),
            "quick-tip" => (("Quick tip", "in 15 seconds"), vec![("This is slow", "and you know it", list("What goes wrong")), ("Do this instead", "one small change", check("How to do it")), ("What you get", "every time", stat("Time saved", "10×", "A number worth showing"))], ("Your product", "Try it free")),
            "did-you-know" => (("Did you know?", "this one fact"), vec![("A surprising fact", "goes here", stat("The fact", "73%", "What the figure counts")), ("Why it is so", "in one line", list("The reasons")), ("What to do", "with it", check("Try this"))], ("Your channel", "Follow for more")),
            "before-after" => (("Before", "and after"), vec![("The old way", "slow and manual", json!({ "type": "compare", "left": { "label": "Steps", "value": "12" }, "right": { "label": "Time", "value": "2 h" } })), ("The new way", "one step", json!({ "type": "compare", "left": { "label": "Steps", "value": "1" }, "right": { "label": "Time", "value": "5 min" } })), ("What changed", "the result", stat("Faster", "24×", "From two hours to five minutes"))], ("Your product", "Try it free")),
            "testimonial" => (("What they say", "about us"), vec![("In their words", "a real customer", json!({ "type": "quote", "text": "It saved us\nhours every week.", "by": "Name, role at Company" })), ("The result", "they got", stat("In three months", "3×", "What grew"))], ("Your product", "Read more stories")),
            "sale" => (("Big sale", "this week only"), vec![("Save big", "on everything", json!({ "type": "price", "was": "$49", "value": "$29", "caption": "Save 40%" })), ("What you get", "with it", check("Included")), ("Ends soon", "do not miss it", stat("Days left", "3", "Ends on Sunday"))], ("Your shop", "Shop now")),
            _ => (("Your feature", "in one line"), vec![("What it does", "in a few words", check("What it does")), ("Why it matters", "to your viewer", stat("The difference", "10×", "A number worth showing")), ("How it looks", "in the product", window)], ("Your product", "Try it free")),
        };
        json!({
            "template": self.key,
            "name": self.label,
            "hook": { "line1": hook.0, "line2": hook.1 },
            "beats": beats.iter().map(|(a, b, card)| json!({ "line1": a, "line2": b, "card": card })).collect::<Vec<_>>(),
            "end": { "title": end.0, "tagline": "Your tagline", "cta": end.1, "url": "yourwebsite.com" },
        })
    }
}

/// How to give a card, for the tool descriptions.
pub fn card_guide() -> String {
    format!(
        "A card is best given as an object ScreenForge lays out, because then it is made of separate elements (each shape an overlay, each piece of text a title) that the user can select, move, retype or delete one by one in the app: \
{{ type: \"list\" or \"checklist\", label, rows: one to four short lines }}; {{ type: \"stat\", label, value: a figure or a word or two, caption }}; {{ type: \"quote\", text (\\n for a second line), by }}; \
{{ type: \"compare\", label, left: {{ label, value }}, right: {{ label, value }} }} (the right side is highlighted); {{ type: \"price\", was, value, caption }}; {{ type: \"window\", label }} (a mock app window). Keep every text to a few words. \
For an illustration none of these fits, give SVG markup instead: <svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {CARD_W} {CARD_H}\">…</svg>, transparent outside what you draw, in the template's colours, text in font-family Inter or Montserrat at 34 px or more. It is then one image: its words cannot be retyped in the app, so prefer shapes and leave the words to the headline."
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
    fn every_template_draws_and_its_cards_are_separate_elements() {
        let mut keys: Vec<&str> = TEMPLATES.iter().map(|t| t.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), TEMPLATES.len());
        // TikTok leaves x 60–940 and y 220–1440 clear; the card canvas has the card area's proportions
        let [x, y, cw, ch] = CARD;
        assert!(x * WIDTH >= 60.0 && (x + cw) * WIDTH <= 940.0 && y * HEIGHT >= 220.0 && (y + ch) * HEIGHT <= 1440.0);
        assert!(((cw * WIDTH / (ch * HEIGHT)) / (CARD_W / CARD_H) - 1.0).abs() < 0.01);
        for t in &TEMPLATES {
            let (png, w, h) = crate::svg::render(&t.background_svg(), 1920).unwrap_or_else(|e| panic!("{}: {e}", t.key));
            assert_eq!((w, h), (1080, 1920), "{}", t.key);
            assert!(png.len() > 1000);
            assert!(find(&t.key.to_uppercase()).is_some());
            // the stand-in project: every card is shapes that draw and words inside the canvas
            let sample = t.sample();
            let beats = sample["beats"].as_array().unwrap();
            assert!((2..=3).contains(&beats.len()), "{}", t.key);
            for beat in beats {
                let els = t.card_elements(&beat["card"]).unwrap_or_else(|e| panic!("{}: {e}", t.key));
                assert!(els.iter().any(|e| matches!(e, Element::Text { .. })), "{}: a card has words to retype", t.key);
                for e in &els {
                    match e {
                        Element::Shape { svg, x, y, w, h } => {
                            assert!(crate::svg::render(svg, 400).is_ok(), "{}: {svg}", t.key);
                            assert!(!svg.contains("</text>") || svg.contains("“") || svg.contains(">1<") || svg.contains(">2<") || svg.contains(">3<"), "{}: only a mark or a number is drawn into a shape", t.key);
                            assert!(*x >= 0.0 && *y >= 0.0 && x + w <= CARD_W && y + h <= CARD_H, "{}: shape inside the card", t.key);
                        }
                        Element::Text { text, x, y, .. } => assert!(!text.is_empty() && *x > 0.0 && *x < CARD_W && *y > 0.0 && *y < CARD_H, "{}: {text}", t.key),
                    }
                }
            }
            assert_eq!(t.button_elements("Try it free").len(), 2);
        }
        assert_eq!(TEMPLATES[1].beat_label(1).as_deref(), Some("STEP 2"));
        assert_eq!(TEMPLATES[3].beat_label(0).as_deref(), Some("NUMBER 3"));
        assert_eq!(TEMPLATES[3].beat_label(5), None);
        assert!(TEMPLATES[0].card_elements(&json!({ "type": "pie" })).is_err());
        assert!(TEMPLATES[0].card_elements(&json!({ "type": "list" })).is_err());
        assert_eq!(file_stem("  My: demo/video?  "), "My demo video");
        assert_eq!(escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
    }
}
