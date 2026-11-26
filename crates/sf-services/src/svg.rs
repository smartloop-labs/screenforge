//! Vector artwork written as SVG (a card or an illustration an agent draws, a template's
//! background) drawn to a PNG, so it can be shown over the video like any other image.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};
use sf_project::tracks::MediaItem;

/// The most SVG text taken (an illustration is a few KB).
pub const MAX_BYTES: usize = 400_000;

/// System fonts plus the app's own Inter and Montserrat, so text in a card looks the same on
/// every computer when it names one of those two.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.load_font_data(include_bytes!("../../sf-render/fonts/Inter-SemiBold-latin.ttf").to_vec());
        db.load_font_data(include_bytes!("../../sf-render/fonts/Montserrat-ExtraBold-latin.ttf").to_vec());
        // the bundled subsets call themselves "Montserrat Thin" (an instance of the variable
        // font) and "Inter" at weight 600: offer them under the plain names at every weight an
        // SVG is likely to ask for, so `font-family="Montserrat"` is never a serif fallback
        let own: Vec<usvg::fontdb::FaceInfo> = db.faces().filter(|f| f.families.iter().any(|(n, _)| n.starts_with("Montserrat") || n == "Inter")).cloned().collect();
        for face in own {
            let family = if face.families.iter().any(|(n, _)| n.starts_with("Montserrat")) { "Montserrat" } else { "Inter" };
            for weight in [400, 500, 600, 700, 800, 900] {
                let mut alias = face.clone();
                alias.families = vec![(family.to_string(), usvg::fontdb::Language::English_UnitedStates)];
                alias.weight = usvg::fontdb::Weight(weight);
                alias.style = usvg::fontdb::Style::Normal;
                db.push_face_info(alias);
            }
        }
        db.set_sans_serif_family("Inter");
        Arc::new(db)
    })
    .clone()
}

/// Draw `svg` to a PNG whose longer side is `side` pixels (its own size when that is larger,
/// up to 4096). Returns the PNG and its size.
pub fn render(svg: &str, side: u32) -> Result<(Vec<u8>, u32, u32), String> {
    if svg.len() > MAX_BYTES {
        return Err(format!("the SVG is too large ({} KB; at most {} KB)", svg.len() / 1000, MAX_BYTES / 1000));
    }
    let opt = usvg::Options { fontdb: fonts(), font_family: "Inter".into(), ..usvg::Options::default() };
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| format!("that is not SVG that can be drawn: {e}"))?;
    let (w, h) = (tree.size().width() as f64, tree.size().height() as f64);
    if w < 1.0 || h < 1.0 {
        return Err("the SVG has no size: give it a viewBox or a width and height".into());
    }
    let scale = (side as f64 / w.max(h)).max(1.0).min(4096.0 / w.max(h));
    let (pw, ph) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("the SVG is too large to draw")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale as f32, scale as f32), &mut pixmap.as_mut());
    let png = pixmap.encode_png().map_err(|e| format!("could not write the image: {e}"))?;
    Ok((png, pw, ph))
}

/// Draw `svg` and add it to the project's media as an image, for an overlay.
pub fn import(package: &Path, svg: &str) -> Result<MediaItem, String> {
    let (png, _, _) = render(svg, 1600)?;
    // named by what it shows, so the same artwork is one file
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in svg.as_bytes() {
        hash = (hash ^ *b as u64).wrapping_mul(0x100000001b3);
    }
    let tmp = std::env::temp_dir().join(format!("card-{:08x}.png", hash as u32));
    std::fs::write(&tmp, png).map_err(|e| format!("could not write the image: {e}"))?;
    let item = crate::media::import(package, &tmp);
    let _ = std::fs::remove_file(&tmp);
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_is_drawn_at_the_asked_size_with_its_transparency() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect x="0" y="0" width="100" height="100" fill="#ff0000"/><text x="110" y="60" font-family="Inter" font-size="30" fill="#ffffff">Hi</text></svg>"##;
        let (png, w, h) = render(svg, 800).unwrap();
        assert_eq!((w, h), (800, 400));
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(100, 200).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(790, 390).0[3], 0, "nothing drawn there: transparent");
        // the text drew something in the right half
        assert!((400..800).any(|x| (150..300).any(|y| img.get_pixel(x, y).0[3] > 0)));
        assert!(render("<svg", 100).is_err());
        assert!(render("not svg at all", 100).is_err());
    }
}
