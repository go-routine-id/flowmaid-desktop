//! Pelukis diagram berbasis `Scene`: node, edge, label, tabel ER, dan
//! box class — dipakai kanvas utama maupun pratinjau inline Markdown.

use crate::paint_static::{draw_journey, draw_mindmap, draw_pie, draw_sequence};
use crate::theme::{accent_color, hex, zfont, EDGE, LABEL_BORDER, TEXT, TYPE_MUTED};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Stroke, Vec2};
use flowmaid::class::{self, ClassBox, RelStyle};
use flowmaid::er::{self, ErTable};
use flowmaid::model::{Card, EdgeKind, Shape};
use flowmaid::pie::{self, PieScene};
use flowmaid::scene::{Scene, SceneNode};
use flowmaid::seq::SeqScene;

/// Referensi data gambar SATU diagram — dipinjam dari field App
/// (kanvas utama) atau dari blok Markdown (pratinjau inline), supaya
/// keduanya memakai satu fungsi lukis yang sama: [`paint_diagram`].
pub(crate) struct DiagramRefs<'a> {
    pub(crate) scn: &'a Scene,
    pub(crate) tables: &'a [ErTable],
    pub(crate) cards: &'a [(Card, Card)],
    pub(crate) boxes: &'a [ClassBox],
    pub(crate) rels: &'a [RelStyle],
    pub(crate) pie: Option<&'a PieScene>,
    pub(crate) pie_labels: &'a [Option<String>],
    pub(crate) pie_empty: bool,
    pub(crate) seq: Option<&'a SeqScene>,
    pub(crate) seq_labels: &'a [String],
}

/// Lukis satu diagram lengkap (kertas, cluster, edge, node/tabel/
/// box, pie, sequence) lewat transform `ts` — dipakai kanvas utama
/// dan pratinjau inline dokumen Markdown.
pub(crate) fn paint_diagram(
    painter: &egui::Painter,
    d: &DiagramRefs,
    hovered: Option<usize>,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
    paper: bool,
) {
    // "Kertas" putih — cermin latar putih ekspor SVG; batasnya bbox
    // konten aktual supaya node yang digeser negatif tetap tertutup.
    if paper && d.scn.width > 0.0 && d.scn.height > 0.0 {
        let (mut x0, mut y0, mut x1, mut y1) = (0.0f64, 0.0f64, d.scn.width, d.scn.height);
        for n in &d.scn.nodes {
            x0 = x0.min(n.x - n.w / 2.0);
            y0 = y0.min(n.y - n.h / 2.0);
            x1 = x1.max(n.x + n.w / 2.0);
            y1 = y1.max(n.y + n.h / 2.0);
        }
        for c in &d.scn.clusters {
            x0 = x0.min(c.x);
            y0 = y0.min(c.y);
        }
        let paper = Rect::from_min_max(ts(x0, y0), ts(x1, y1)).expand(16.0 * zoom);
        painter.rect_filled(
            paper.translate(Vec2::new(0.0, 2.5)).expand(1.5),
            9.0 * zoom,
            Color32::from_black_alpha(70),
        );
        painter.rect(paper, 8.0 * zoom, Color32::WHITE, Stroke::new(1.0_f32, LABEL_BORDER));
    }

    for c in &d.scn.clusters {
        let tl = ts(c.x, c.y);
        let rect = Rect::from_min_size(tl, Vec2::new(c.w as f32, c.h as f32) * zoom);
        painter.rect(
            rect,
            8.0 * zoom,
            Color32::from_rgb(0xf7, 0xf8, 0xfd),
            Stroke::new(1.4 * zoom, Color32::from_rgb(0xc9, 0xcf, 0xe8)),
        );
        painter.text(
            tl + Vec2::new(10.0, 11.0) * zoom,
            Align2::LEFT_CENTER,
            &c.title,
            // Jaga agar judul tetap terbaca saat zoom kecil
            // (12 × 0.5 = lantai 6 pt, tetap terkuantisasi).
            zfont(12.0, zoom.max(0.5)),
            TYPE_MUTED,
        );
    }
    let is_er = !d.tables.is_empty();
    let is_class = !d.boxes.is_empty();
    for (i, e) in d.scn.edges.iter().enumerate() {
        if matches!(e.kind, EdgeKind::Invisible) {
            continue; // link penata layout — tidak digambar
        }
        let sw = (if matches!(e.kind, EdgeKind::Thick | EdgeKind::ThickOpen) {
            3.4
        } else {
            1.7
        }) * zoom;
        let stroke = Stroke::new(sw, EDGE);
        let dotted = matches!(e.kind, EdgeKind::Dotted | EdgeKind::DottedOpen);
        if !e.waypoints.is_empty() {
            // Edge panjang di-route lewat channel virtual-node: spline
            // Catmull-Rom melalui waypoint-nya (mirip mermaid). Hanya
            // flowchart yang punya waypoint (bukan class/ER), jadi cukup
            // panah di ujung — tanpa glyph.
            let pts: Vec<Pos2> = e.waypoints.iter().map(|&(x, y)| ts(x, y)).collect();
            draw_spline(painter, &pts, stroke, dotted);
            if e.kind.has_arrow() {
                let n = pts.len();
                arrow_head(painter, [pts[n - 2], pts[n - 2], pts[n - 2], pts[n - 1]], EDGE, zoom);
            }
        } else {
            let p = e.bezier.map(|(x, y)| ts(x, y));
            if dotted {
                dashed_bezier(painter, p, stroke);
            } else {
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                    p,
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            }
            if is_class {
                // Glyph UML di ujung `to`; kardinalitas di kedua sisi.
                // `.get` menjaga andai edge & rels tak sejajar.
                if let Some(rel) = d.rels.get(i) {
                    draw_head(painter, &class::head(e.bezier[3], e.bezier[2], rel.kind), ts, zoom);
                    if let Some(c) = &rel.from_card {
                        draw_card(painter, e.bezier[0], e.bezier[1], c, ts, zoom);
                    }
                    if let Some(c) = &rel.to_card {
                        draw_card(painter, e.bezier[3], e.bezier[2], c, ts, zoom);
                    }
                }
            } else if let Some(&(cf, ct)) = d.cards.get(i).filter(|_| is_er) {
                // Notasi crow's foot di kedua ujung relasi ER.
                draw_glyph(painter, &er::glyph(e.bezier[0], e.bezier[1], cf), ts, zoom);
                draw_glyph(painter, &er::glyph(e.bezier[3], e.bezier[2], ct), ts, zoom);
            } else if e.kind.has_arrow() {
                arrow_head(painter, p, EDGE, zoom);
            }
        }
        if let Some((t, (lx, ly), lw)) = &e.label {
            let c = ts(*lx, *ly);
            let r = Rect::from_center_size(c, Vec2::new(*lw as f32, 20.0) * zoom);
            painter.rect(r, 4.0 * zoom, Color32::WHITE, Stroke::new(1.0 * zoom, LABEL_BORDER));
            draw_rich_text(painter, c, t, 13.0, zoom, TEXT);
        }
    }
    if is_class {
        for (i, (n, b)) in d.scn.nodes.iter().zip(d.boxes).enumerate() {
            draw_class_box(painter, n, b, ts(n.x, n.y), zoom, accent_color(i), hovered == Some(i));
        }
    } else if is_er {
        for (i, (n, t)) in d.scn.nodes.iter().zip(d.tables).enumerate() {
            draw_table(painter, n, t, ts(n.x, n.y), zoom, accent_color(i), hovered == Some(i));
        }
    } else {
        for (i, n) in d.scn.nodes.iter().enumerate() {
            draw_node(painter, n, ts(n.x, n.y), zoom, hovered == Some(i));
        }
    }
    if let Some(ps) = d.pie {
        draw_pie(painter, ps, d.pie_labels, d.pie_empty, ts, zoom);
    } else if let Some(sq) = d.seq {
        draw_sequence(painter, sq, d.seq_labels, ts, zoom);
    }
}

/// `DiagramRefs` untuk diagram tanpa data samping (flowchart/state, dan
/// dasar bagi pie/sequence yang scene-nya kosong). Lifetime elision
/// mengikat hasilnya ke `scn` — tak bisa ditulis sebagai closure.
fn base_refs(scn: &Scene) -> DiagramRefs<'_> {
    DiagramRefs {
        scn,
        tables: &[],
        cards: &[],
        boxes: &[],
        rels: &[],
        pie: None,
        pie_labels: &[],
        pie_empty: false,
        seq: None,
        seq_labels: &[],
    }
}

/// Lukis satu diagram tertanam dari sebuah `DiagramView` markmaid lewat
/// [`paint_diagram`] bersama, dengan transform `ts`. Data turunan (label
/// pie/sequence) dihitung on the fly — `DiagramView` menyimpan scene
/// flowmaid apa adanya.
pub(crate) fn paint_embedded(
    painter: &egui::Painter,
    view: &markmaid::DiagramView,
    blank: &Scene,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    use markmaid::DiagramView as V;
    match view {
        V::Flow(s) => paint_diagram(painter, &base_refs(s), None, ts, zoom, false),
        V::Er(es) => {
            let refs = DiagramRefs {
                tables: &es.tables,
                cards: &es.cards,
                ..base_refs(&es.scene)
            };
            paint_diagram(painter, &refs, None, ts, zoom, false);
        }
        V::Class(cs) => {
            let refs = DiagramRefs {
                boxes: &cs.boxes,
                rels: &cs.rels,
                ..base_refs(&cs.scene)
            };
            paint_diagram(painter, &refs, None, ts, zoom, false);
        }
        V::Pie(ps) => {
            let pie_labels: Vec<Option<String>> = ps
                .slices
                .iter()
                .map(|sl| {
                    (sl.frac >= pie::MIN_LABEL_FRAC).then(|| format!("{:.0}%", sl.frac * 100.0))
                })
                .collect();
            let pie_empty = ps.slices.iter().map(|s| s.frac).sum::<f64>() <= f64::EPSILON;
            let refs = DiagramRefs {
                pie: Some(ps),
                pie_labels: &pie_labels,
                pie_empty,
                ..base_refs(blank)
            };
            paint_diagram(painter, &refs, None, ts, zoom, false);
        }
        V::Seq(ss) => {
            let seq_labels: Vec<String> = ss
                .messages
                .iter()
                .map(|m| match m.number {
                    Some(k) => format!("{k}. {}", m.text),
                    None => m.text.clone(),
                })
                .collect();
            let refs = DiagramRefs {
                seq: Some(ss),
                seq_labels: &seq_labels,
                ..base_refs(blank)
            };
            paint_diagram(painter, &refs, None, ts, zoom, false);
        }
        // Mindmap & journey tak lewat DiagramRefs (bukan Scene node/
        // edge) — punya painter sendiri.
        V::Mind(ms) => draw_mindmap(painter, ms, ts, zoom),
        V::Journey(jsc) => draw_journey(painter, jsc, ts, zoom),
        // Static topologi (node/edge/cluster) lewat `Scene` generik.
        V::Git(gs) => paint_diagram(painter, &base_refs(&gs.scene), None, ts, zoom, false),
        V::Arch(as_) => paint_diagram(painter, &base_refs(&as_.scene), None, ts, zoom, false),
    }
}

fn draw_node(p: &egui::Painter, n: &SceneNode, c: Pos2, zoom: f32, hovered: bool) {
    // Tema per-bentuk, ditimpa style/classDef kustom dari teks.
    let ss = flowmaid::style::shape_style(n.shape);
    let fill = hex(n.style.fill.as_deref().unwrap_or(ss.fill));
    let base_w = n.style.stroke_width.unwrap_or(1.6) as f32;
    let (w, h) = (n.w as f32 * zoom, n.h as f32 * zoom);
    let stroke = Stroke::new(
        (if hovered { base_w + 1.2 } else { base_w }) * zoom,
        hex(n.style.stroke.as_deref().unwrap_or(ss.stroke)),
    );
    let text_color = n.style.color.as_deref().map(hex).unwrap_or(TEXT);
    let poly = |pts: Vec<Pos2>| egui::epaint::PathShape::convex_polygon(pts, fill, stroke);
    let (hw, hh) = (w / 2.0, h / 2.0);
    match n.shape {
        // Pseudostate stateDiagram — cermin persis SVG-nya.
        Shape::StateStart => {
            p.circle_filled(c, hw, fill);
            return; // tanpa label
        }
        Shape::StateEnd => {
            p.circle(c, hw, Color32::WHITE, stroke);
            p.circle_filled(c, (hw - 4.0 * zoom).max(2.0), fill);
            return;
        }
        Shape::ForkBar => {
            p.rect_filled(Rect::from_center_size(c, Vec2::new(w, h)), 3.0 * zoom, fill);
            return;
        }
        Shape::Circle => {
            p.circle(c, hw, fill, stroke);
        }
        Shape::DoubleCircle => {
            p.circle(c, hw, fill, stroke);
            p.circle(c, hw - 4.0 * zoom, Color32::TRANSPARENT, stroke);
        }
        Shape::Diamond => {
            p.add(poly(vec![
                Pos2::new(c.x, c.y - hh),
                Pos2::new(c.x + hw, c.y),
                Pos2::new(c.x, c.y + hh),
                Pos2::new(c.x - hw, c.y),
            ]));
        }
        Shape::Hexagon => {
            let k = (14.0 * zoom).min(w / 4.0);
            p.add(poly(vec![
                Pos2::new(c.x - hw, c.y),
                Pos2::new(c.x - hw + k, c.y - hh),
                Pos2::new(c.x + hw - k, c.y - hh),
                Pos2::new(c.x + hw, c.y),
                Pos2::new(c.x + hw - k, c.y + hh),
                Pos2::new(c.x - hw + k, c.y + hh),
            ]));
        }
        Shape::Parallelogram | Shape::ParallelogramAlt => {
            let k = (14.0 * zoom).min(w / 4.0);
            let pts = if matches!(n.shape, Shape::Parallelogram) {
                vec![
                    Pos2::new(c.x - hw + k, c.y - hh),
                    Pos2::new(c.x + hw, c.y - hh),
                    Pos2::new(c.x + hw - k, c.y + hh),
                    Pos2::new(c.x - hw, c.y + hh),
                ]
            } else {
                vec![
                    Pos2::new(c.x - hw, c.y - hh),
                    Pos2::new(c.x + hw - k, c.y - hh),
                    Pos2::new(c.x + hw, c.y + hh),
                    Pos2::new(c.x - hw + k, c.y + hh),
                ]
            };
            p.add(poly(pts));
        }
        Shape::Cylinder => {
            let ry = (8.0 * zoom).min(h / 4.0);
            // Body (rounded top/bottom approximates the caps) + a
            // top arc line for the database look.
            let body = Rect::from_center_size(c, Vec2::new(w, h - ry));
            p.rect(body, ry, fill, stroke);
            let top = c.y - hh + ry;
            p.line_segment(
                [Pos2::new(c.x - hw, top), Pos2::new(c.x + hw, top)],
                stroke,
            );
        }
        Shape::Subroutine => {
            let r = Rect::from_center_size(c, Vec2::new(w, h));
            p.rect(r, 3.0 * zoom, fill, stroke);
            for dx in [-hw + 8.0 * zoom, hw - 8.0 * zoom] {
                p.line_segment(
                    [Pos2::new(c.x + dx, c.y - hh), Pos2::new(c.x + dx, c.y + hh)],
                    stroke,
                );
            }
        }
        _ => {
            let r = Rect::from_center_size(c, Vec2::new(w, h));
            let round = match n.shape {
                Shape::Rounded => 9.0 * zoom,
                Shape::Stadium => hh,
                _ => 3.0 * zoom,
            };
            p.rect(r, round, fill, stroke);
        }
    }
    draw_rich_text(p, c, &n.label, 14.0, zoom, text_color);
}

/// Lukis label (multi-baris via '\n') ter-tengah di `c`; run `<b>`/`<i>`
/// dari `flowmaid::layout::spans` digambar tebal — padanan tspan bold
/// di SVG engine. egui tak membawa font bold, jadi bold di-faux dengan
/// menggambar galley dua kali bergeser sub-piksel.
fn draw_rich_text(
    p: &egui::Painter,
    c: Pos2,
    label: &str,
    size: f32,
    zoom: f32,
    color: Color32,
) {
    let lines: Vec<&str> = label.split('\n').collect();
    let line_h = flowmaid::layout::LINE_H as f32 * zoom;
    let font = zfont(size, zoom);
    for (i, line) in lines.iter().enumerate() {
        let y = c.y + (i as f32 - (lines.len() as f32 - 1.0) / 2.0) * line_h;
        let runs = flowmaid::layout::spans(line);
        if runs.len() == 1 && !runs[0].1 && !runs[0].2 {
            p.text(
                Pos2::new(c.x, y),
                Align2::CENTER_CENTER,
                &runs[0].0,
                font.clone(),
                color,
            );
            continue;
        }
        let galleys: Vec<_> = runs
            .iter()
            .map(|(t, _, _)| p.fonts(|f| f.layout_no_wrap(t.clone(), font.clone(), color)))
            .collect();
        let total: f32 = galleys.iter().map(|g| g.size().x).sum();
        let mut x = c.x - total / 2.0;
        for (g, (_, bold, _)) in galleys.into_iter().zip(&runs) {
            let pos = Pos2::new(x, y - g.size().y / 2.0);
            if *bold {
                p.galley(pos + Vec2::new(0.4 * zoom.max(1.0), 0.0), g.clone(), color);
            }
            x += g.size().x;
            p.galley(pos, g, color);
        }
    }
}

/// Tabel entitas ER: header berwarna + baris atribut
/// (tipe redup | nama | tag kunci rata kanan).
fn draw_table(
    p: &egui::Painter,
    n: &SceneNode,
    t: &ErTable,
    c: Pos2,
    zoom: f32,
    accent: Color32,
    hovered: bool,
) {
    use flowmaid::er::{COL_GAP, HEADER_H, PAD, ROW_H};
    let (w, h) = (n.w as f32 * zoom, n.h as f32 * zoom);
    let x0 = c.x - w / 2.0;
    let y0 = c.y - h / 2.0;
    let round = 4.0 * zoom;
    p.rect(
        Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(w, h)),
        round,
        Color32::WHITE,
        Stroke::new(if hovered { 2.8 } else { 1.6 } * zoom, accent),
    );
    let hh = HEADER_H as f32 * zoom;
    p.rect(
        Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(w, hh)),
        egui::Rounding {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        accent,
        Stroke::NONE,
    );
    p.text(
        Pos2::new(c.x, y0 + hh / 2.0),
        Align2::CENTER_CENTER,
        &t.name,
        zfont(13.5, zoom),
        Color32::WHITE,
    );
    let row_h = ROW_H as f32 * zoom;
    for (i, row) in t.rows.iter().enumerate() {
        let ry = y0 + hh + i as f32 * row_h;
        if i > 0 {
            p.line_segment(
                [Pos2::new(x0, ry), Pos2::new(x0 + w, ry)],
                Stroke::new(1.0 * zoom, LABEL_BORDER),
            );
        }
        let cy = ry + row_h / 2.0;
        let f = zfont(12.5, zoom);
        p.text(
            Pos2::new(x0 + PAD as f32 * zoom, cy),
            Align2::LEFT_CENTER,
            &row.ty,
            f.clone(),
            TYPE_MUTED,
        );
        p.text(
            Pos2::new(x0 + (PAD + t.ty_col_w + COL_GAP) as f32 * zoom, cy),
            Align2::LEFT_CENTER,
            &row.name,
            f.clone(),
            TEXT,
        );
        if !row.keys.is_empty() {
            p.text(
                Pos2::new(x0 + w - PAD as f32 * zoom, cy),
                Align2::RIGHT_CENTER,
                &row.keys,
                f,
                EDGE,
            );
        }
    }
}

/// Glyph crow's foot (segmen garis + lingkaran opsional) dalam
/// koordinat dunia, ditransformasikan ke layar saat digambar.
fn draw_glyph(
    p: &egui::Painter,
    g: &flowmaid::er::Glyph,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    let stroke = Stroke::new(1.7 * zoom, EDGE);
    for [a, b] in &g.segments {
        p.line_segment([ts(a.0, a.1), ts(b.0, b.1)], stroke);
    }
    if let Some((c, r)) = g.circle {
        p.circle(ts(c.0, c.1), r as f32 * zoom, Color32::WHITE, stroke);
    }
}

/// Box class tiga kompartemen (nama / field / method) — cermin dari
/// `class::to_svg`, memakai konstanta ukuran engine yang sama.
#[allow(clippy::too_many_arguments)]
fn draw_class_box(
    p: &egui::Painter,
    n: &SceneNode,
    b: &ClassBox,
    c: Pos2,
    zoom: f32,
    accent: Color32,
    hovered: bool,
) {
    use flowmaid::class::{ClassRow, EMPTY_H, HEADER_H, PAD, ROW_H};
    let (w, h) = (n.w as f32 * zoom, n.h as f32 * zoom);
    let x0 = c.x - w / 2.0;
    let y0 = c.y - h / 2.0;
    let round = 4.0 * zoom;
    p.rect(
        Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(w, h)),
        round,
        Color32::WHITE,
        Stroke::new(if hovered { 2.8 } else { 1.6 } * zoom, accent),
    );
    // Kompartemen nama (header accent, sudut atas membulat).
    let hh = HEADER_H as f32 * zoom;
    p.rect(
        Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(w, hh)),
        egui::Rounding {
            nw: round,
            ne: round,
            sw: 0.0,
            se: 0.0,
        },
        accent,
        Stroke::NONE,
    );
    p.text(
        Pos2::new(c.x, y0 + hh / 2.0),
        Align2::CENTER_CENTER,
        &b.name,
        zfont(13.5, zoom),
        Color32::WHITE,
    );
    let row_h = ROW_H as f32 * zoom;
    let font = zfont(12.5, zoom);
    let comp_h =
        |rows: usize| (if rows == 0 { EMPTY_H as f32 } else { rows as f32 * ROW_H as f32 }) * zoom;
    // Pemisah + baris kompartemen, mulai dari `top`.
    let draw_rows = |top: f32, rows: &[ClassRow]| {
        p.line_segment(
            [Pos2::new(x0, top), Pos2::new(x0 + w, top)],
            Stroke::new(1.0 * zoom, LABEL_BORDER),
        );
        for (i, row) in rows.iter().enumerate() {
            let cy = top + i as f32 * row_h + row_h / 2.0;
            p.text(
                Pos2::new(x0 + PAD as f32 * zoom, cy),
                Align2::LEFT_CENTER,
                &row.text,
                font.clone(),
                TEXT,
            );
        }
    };
    let fields_top = y0 + hh;
    draw_rows(fields_top, &b.fields);
    draw_rows(fields_top + comp_h(b.fields.len()), &b.methods);
}

/// Glyph ujung UML (segitiga/diamond terisi-atau-hollow / panah
/// terbuka) dalam koordinat dunia, ditransformasikan saat digambar.
fn draw_head(
    p: &egui::Painter,
    h: &class::Head,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    let stroke = Stroke::new(1.6 * zoom, EDGE);
    if !h.polygon.is_empty() {
        let pts: Vec<Pos2> = h.polygon.iter().map(|(x, y)| ts(*x, *y)).collect();
        let fill = if h.filled { EDGE } else { Color32::WHITE };
        p.add(egui::epaint::PathShape::convex_polygon(pts, fill, stroke));
    }
    for [a, b] in &h.segments {
        p.line_segment([ts(a.0, a.1), ts(b.0, b.1)], stroke);
    }
}

/// Label kardinalitas class, sedikit ke dalam & menyamping dari ujung.
fn draw_card(
    p: &egui::Painter,
    e: (f64, f64),
    c: (f64, f64),
    text: &str,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    let (dx, dy) = (c.0 - e.0, c.1 - e.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ux, uy) = (dx / len, dy / len);
    let px = e.0 + ux * 14.0 - uy * 9.0;
    let py = e.1 + uy * 14.0 + ux * 9.0;
    p.text(
        ts(px, py),
        Align2::CENTER_CENTER,
        text,
        zfont(11.0, zoom),
        TEXT,
    );
}

/// Scene kosong dengan ukuran kanvas — dipakai diagram statis
/// (pie/sequence) yang tak punya node yang bisa digeser.
pub(crate) fn blank_scene(width: f64, height: f64) -> Scene {
    Scene::empty(width, height)
}

/// Garis putus-putus lurus (egui tak punya dash bawaan) dalam
/// koordinat layar. `dash`/`gap` dalam piksel layar — pemanggil
/// mengalikan zoom supaya ritme dash cocok dengan `stroke-dasharray`
/// SVG pada level zoom berapa pun (paritas per elemen: lifeline 4/4,
/// divider 5/4, pesan 6/4 — temuan bug hunt).
pub(crate) fn dashed_line(p: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, dash: f32, gap: f32) {
    let len = (b - a).length();
    let dir = (b - a) / len.max(0.001);
    let step = (dash + gap).max(0.5); // jaga-jaga zoom ekstrem kecil
    let mut d = 0.0;
    while d < len {
        let s1 = a + dir * (d + dash).min(len);
        p.line_segment([a + dir * d, s1], stroke);
        d += step;
    }
}

/// Gambar B-spline `curveBasis` (kurva d3 yang dipakai mermaid)
/// melalui `pts` (>= 2 titik): mulai & berakhir tepat di ujung,
/// waypoint tengah hanya DIDEKATI sehingga edge mengalir dalam jalur
/// halus, tidak menggembung melewati tiap titik. Matematikanya sama
/// dengan `spline_d` engine agar kanvas & ekspor SVG identik.
fn draw_spline(p: &egui::Painter, pts: &[Pos2], stroke: Stroke, dotted: bool) {
    let n = pts.len();
    let seg_line = |p: &egui::Painter, a: Pos2, b: Pos2| {
        if dotted {
            dashed_bezier(p, [a, a, b, b], stroke);
        } else {
            p.line_segment([a, b], stroke);
        }
    };
    if n < 3 {
        seg_line(p, pts[0], pts[n - 1]);
        return;
    }
    seg_line(
        p,
        pts[0],
        Pos2::new(
            (5.0 * pts[0].x + pts[1].x) / 6.0,
            (5.0 * pts[0].y + pts[1].y) / 6.0,
        ),
    );
    // Kontrol bezier satu segmen basis untuk trio (a, b, q).
    let bez = |a: Pos2, b: Pos2, q: Pos2| {
        let c1 = Pos2::new((2.0 * a.x + b.x) / 3.0, (2.0 * a.y + b.y) / 3.0);
        let c2 = Pos2::new((a.x + 2.0 * b.x) / 3.0, (a.y + 2.0 * b.y) / 3.0);
        let end = Pos2::new((a.x + 4.0 * b.x + q.x) / 6.0, (a.y + 4.0 * b.y + q.y) / 6.0);
        (c1, c2, end)
    };
    // Titik awal segmen pertama = hasil lineTo di atas; tiap segmen
    // menyambung mulus karena B-spline C2-continuous.
    let mut cur = Pos2::new(
        (5.0 * pts[0].x + pts[1].x) / 6.0,
        (5.0 * pts[0].y + pts[1].y) / 6.0,
    );
    for i in 2..=n {
        let (a, b, q) = if i < n {
            (pts[i - 2], pts[i - 1], pts[i])
        } else {
            (pts[n - 2], pts[n - 1], pts[n - 1])
        };
        let (c1, c2, end) = bez(a, b, q);
        let seg = [cur, c1, c2, end];
        if dotted {
            dashed_bezier(p, seg, stroke);
        } else {
            p.add(egui::epaint::CubicBezierShape::from_points_stroke(
                seg,
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
        }
        cur = end;
    }
    seg_line(p, cur, pts[n - 1]);
}

fn arrow_head(p: &egui::Painter, b: [Pos2; 4], color: Color32, zoom: f32) {
    let tip = b[3];
    let d = tip - b[2];
    let len = d.length().max(0.001);
    let dir = d / len;
    let n = Vec2::new(-dir.y, dir.x);
    let back = tip - dir * 9.0 * zoom;
    p.add(egui::epaint::PathShape::convex_polygon(
        vec![tip, back + n * 4.0 * zoom, back - n * 4.0 * zoom],
        color,
        Stroke::NONE,
    ));
}

/// egui tidak punya dash bawaan untuk bezier: sampling manual.
fn dashed_bezier(p: &egui::Painter, b: [Pos2; 4], stroke: Stroke) {
    let f = |t: f32| {
        let u = 1.0 - t;
        Pos2::new(
            u * u * u * b[0].x
                + 3.0 * u * u * t * b[1].x
                + 3.0 * u * t * t * b[2].x
                + t * t * t * b[3].x,
            u * u * u * b[0].y
                + 3.0 * u * u * t * b[1].y
                + 3.0 * u * t * t * b[2].y
                + t * t * t * b[3].y,
        )
    };
    let n = 36;
    let mut prev = f(0.0);
    for k in 1..=n {
        let cur = f(k as f32 / n as f32);
        if k % 2 == 1 {
            p.line_segment([prev, cur], stroke);
        }
        prev = cur;
    }
}
