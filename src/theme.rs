//! Palet warna & pemetaan warna: konstanta tinta diagram, parser
//! warna CSS, dan peran warna markmaid → tema egui aktif.

use eframe::egui::{self, Color32, FontId};

pub(crate) const EDGE: Color32 = Color32::from_rgb(0x44, 0x50, 0x7a);
pub(crate) const TEXT: Color32 = Color32::from_rgb(0x23, 0x28, 0x40);
pub(crate) const LABEL_BORDER: Color32 = Color32::from_rgb(0xd5, 0xd9, 0xec);
pub(crate) const TYPE_MUTED: Color32 = Color32::from_rgb(0x6a, 0x70, 0x86);
// Warna tetap jalur gambar sequence — const, bukan hex() per frame.
pub(crate) const GUIDE: Color32 = Color32::from_rgb(0xae, 0xb6, 0xd8);
pub(crate) const CHIP_FILL: Color32 = Color32::from_rgb(0xee, 0xf1, 0xfb);
pub(crate) const NOTE_FILL: Color32 = Color32::from_rgb(0xfc, 0xf2, 0xda);
pub(crate) const NOTE_STROKE: Color32 = Color32::from_rgb(0xd9, 0x91, 0x14);

/// Warna CSS (tema engine / style user) → Color32, supaya kanvas
/// dan ekspor SVG memakai warna yang persis sama. Mendukung
/// `#rrggbb`, shorthand `#rgb`, dan nama warna CSS yang umum —
/// semuanya bentuk yang diterima renderer SVG.
pub(crate) fn hex(c: &str) -> Color32 {
    let c = c.trim();
    if let Some(h) = c.strip_prefix('#') {
        let expand = |s: &str| -> Option<(u8, u8, u8)> {
            Some((
                u8::from_str_radix(&s[0..2], 16).ok()?,
                u8::from_str_radix(&s[2..4], 16).ok()?,
                u8::from_str_radix(&s[4..6], 16).ok()?,
            ))
        };
        let rgb = match h.len() {
            6 if h.is_ascii() => expand(h),
            // #f9f → #ff99ff, persis aturan CSS.
            3 if h.is_ascii() => {
                let d: Vec<String> = h.chars().map(|ch| format!("{ch}{ch}")).collect();
                expand(&d.concat())
            }
            _ => None,
        };
        if let Some((r, g, b)) = rgb {
            return Color32::from_rgb(r, g, b);
        }
    }
    // Nama warna CSS dasar yang lazim dipakai di diagram mermaid.
    match c.to_ascii_lowercase().as_str() {
        "black" => Color32::from_rgb(0, 0, 0),
        "white" => Color32::from_rgb(255, 255, 255),
        "red" => Color32::from_rgb(255, 0, 0),
        "green" => Color32::from_rgb(0, 128, 0),
        "blue" => Color32::from_rgb(0, 0, 255),
        "yellow" => Color32::from_rgb(255, 255, 0),
        "orange" => Color32::from_rgb(255, 165, 0),
        "purple" => Color32::from_rgb(128, 0, 128),
        "pink" => Color32::from_rgb(255, 192, 203),
        "teal" => Color32::from_rgb(0, 128, 128),
        "cyan" => Color32::from_rgb(0, 255, 255),
        "brown" => Color32::from_rgb(165, 42, 42),
        "lightgray" | "lightgrey" => Color32::from_rgb(211, 211, 211),
        _ => Color32::GRAY,
    }
}

/// FontId proporsional dengan ukuran TERKUANTISASI: kelipatan 0.5 pt,
/// lantai 2 pt. Ukuran f32 kontinu (mis. `13.0 * zoom`) membuat egui
/// merasterisasi set glyph baru untuk TIAP nilai unik dan menaruhnya
/// di atlas font yang tak pernah menyusut — atlas membengkak tanpa
/// batas selama pinch-zoom (temuan audit memory). 0.5 pt = 1 piksel
/// fisik di layar 2x, granularitas terhalus yang epaint render.
pub(crate) fn zfont(base: f32, zoom: f32) -> FontId {
    FontId::proportional(((base * zoom).max(2.0) * 2.0).round() / 2.0)
}

/// Warna accent engine di-parse sekali, bukan `hex()` (parsing string
/// CSS) per elemen per frame.
pub(crate) fn accent_color(i: usize) -> Color32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<Color32>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        (0..flowmaid::style::ACCENTS.len())
            .map(|k| hex(flowmaid::style::accent(k)))
            .collect()
    });
    t[i % t.len()]
}

/// Peran warna markmaid → warna tema egui aktif. markmaid sengaja
/// memakai PERAN, bukan nilai — konsumer memetakannya ke temanya
/// sendiri, jadi dokumen ikut gelap/terang mengikuti app alih-alih
/// memakai palet kertas-putih bawaan engine. `DiagramBg` tetap putih:
/// diagram flowmaid memakai tinta gelap yang mengasumsikan kertas putih.
pub(crate) fn role(v: &egui::Visuals, r: markmaid::ColorRole) -> Color32 {
    use markmaid::ColorRole as R;
    match r {
        R::Text | R::CodeText => v.text_color(),
        R::Strong => v.strong_text_color(),
        R::Muted => v.weak_text_color(),
        R::Link => v.hyperlink_color,
        R::CodeBg => v.code_bg_color,
        R::CodeHighlightBg => Color32::from_rgb(0xdb, 0xe4, 0xff),
        // Syntax highlighting palette — same semantic slots as markmaid's
        // default light-paper SVG palette, adapted slightly for egui themes.
        R::CodeComment => Color32::from_rgb(0x6a, 0x73, 0x7d),
        R::CodeKeyword => Color32::from_rgb(0xd7, 0x3a, 0x49),
        R::CodeString => Color32::from_rgb(0x22, 0x86, 0x3a),
        R::CodeNumber => Color32::from_rgb(0x00, 0x5c, 0xc5),
        R::CodeType => Color32::from_rgb(0x6f, 0x42, 0xc1),
        R::CodeFunction => Color32::from_rgb(0x82, 0x50, 0xdf),
        R::QuoteBg | R::TableStripeBg => v.faint_bg_color,
        R::Border => v.widgets.noninteractive.bg_stroke.color,
        R::ErrorText => v.error_fg_color,
        // Merah semi-transparan → terbaca di tema terang maupun gelap.
        R::ErrorBg => Color32::from_rgba_unmultiplied(0xc9, 0x2a, 0x2a, 40),
        R::DiagramBg => Color32::WHITE,
    }
}
