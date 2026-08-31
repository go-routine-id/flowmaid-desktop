//! Pelukis diagram yang punya geometri sendiri (bukan `Scene` node/edge):
//! mindmap, journey, pie, dan sequence.

use crate::paint::dashed_line;
use crate::theme::{
    accent_color, hex, zfont, CHIP_FILL, EDGE, GUIDE, NOTE_FILL, NOTE_STROKE, TEXT,
};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Stroke, Vec2};
use flowmaid::journey::JourneyScene;
use flowmaid::pie::{self, PieScene};
use flowmaid::seq::{self, SeqScene};

/// Isi sebuah poligon star-shaped (cembung dari pusatnya — bang &
/// cloud) lewat kipas segitiga dari `center`; `convex_polygon` egui
/// salah mengisi bentuk cekung, jadi pakai mesh manual.
fn fill_star(painter: &egui::Painter, center: Pos2, pts: &[Pos2], fill: Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(center, fill);
    for &p in pts {
        mesh.colored_vertex(p, fill);
    }
    let n = pts.len() as u32;
    for k in 0..n {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Lukis satu mindmap ala Mermaid: konektor Bézier meruncing di
/// belakang, lalu node berwarna-isi (bentuk sesuai `MindShape`) dengan
/// label ter-tengah. Geometri & warna dari engine dipakai apa adanya,
/// jadi sama persis dengan ekspor SVG.
pub(crate) fn draw_mindmap(
    painter: &egui::Painter,
    ms: &flowmaid::mindmap::MindScene,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    use flowmaid::model::MindShape;

    // Konektor (di belakang node), meruncing dari akar ke daun.
    for e in &ms.edges {
        let p = e.bezier.map(|(x, y)| ts(x, y));
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            p,
            false,
            Color32::TRANSPARENT,
            Stroke::new(e.width as f32 * zoom, hex(e.color)),
        ));
    }

    for n in &ms.nodes {
        let fill = hex(n.fill);
        // Bentuk poligon (hexagon/bang/cloud) memakai titik dari engine
        // supaya identik dengan SVG; sisanya rect/elips.
        if let Some(poly) = flowmaid::mindmap::perimeter(n) {
            let pts: Vec<Pos2> = poly.iter().map(|&(x, y)| ts(x, y)).collect();
            if n.shape == MindShape::Hexagon {
                painter.add(egui::epaint::PathShape::convex_polygon(pts, fill, Stroke::NONE));
            } else {
                fill_star(painter, ts(n.cx(), n.cy()), &pts, fill);
            }
        } else if n.shape == MindShape::Circle {
            let pts: Vec<Pos2> = (0..32)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / 32.0;
                    ts(n.cx() + n.w / 2.0 * a.cos(), n.cy() + n.h / 2.0 * a.sin())
                })
                .collect();
            painter.add(egui::epaint::PathShape::convex_polygon(pts, fill, Stroke::NONE));
        } else {
            let r = Rect::from_two_pos(ts(n.x, n.y), ts(n.x + n.w, n.y + n.h));
            let round = match n.shape {
                MindShape::Square => 3.0 * zoom,
                _ => (r.height() / 2.0).min(14.0 * zoom),
            };
            painter.rect_filled(r, round, fill);
        }
        // Label ter-tengah, multi-baris via '\n'.
        let color = hex(n.text_color);
        let c = ts(n.cx(), n.cy());
        let lines: Vec<&str> = n.text.split('\n').collect();
        let line_h = flowmaid::mindmap::LINE_H as f32 * zoom;
        for (i, line) in lines.iter().enumerate() {
            let dy = (i as f32 - (lines.len() as f32 - 1.0) / 2.0) * line_h;
            painter.text(
                Pos2::new(c.x, c.y + dy),
                Align2::CENTER_CENTER,
                line,
                zfont(14.0, zoom),
                color,
            );
        }
    }
}

/// Lukis satu user-journey: pita seksi berwarna, garis penghubung,
/// wajah ber-skor (senyum→cemberut), titik aktor, judul, dan legenda.
/// Geometri & warna dari engine dipakai apa adanya (identik dgn SVG).
pub(crate) fn draw_journey(
    painter: &egui::Painter,
    js: &JourneyScene,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    // Pita seksi.
    for b in &js.sections {
        let r = Rect::from_min_max(ts(b.x, b.y), ts(b.x + b.w, b.y + b.h));
        painter.rect_filled(r, 6.0 * zoom, hex(b.color));
        if !b.name.is_empty() {
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                &b.name,
                zfont(13.0, zoom),
                Color32::WHITE,
            );
        }
    }
    // Garis perjalanan (di belakang wajah).
    if js.path.len() >= 2 {
        let pts: Vec<Pos2> = js.path.iter().map(|&(x, y)| ts(x, y)).collect();
        let stroke = Stroke::new(2.0 * zoom, hex(flowmaid::journey::PATH_COLOR));
        painter.add(egui::Shape::line(pts, stroke));
    }
    // Wajah + titik aktor + label.
    for t in &js.tasks {
        for (dx, dy, color) in &t.actor_dots {
            painter.circle_filled(ts(*dx, *dy), flowmaid::journey::DOT_R as f32 * zoom, hex(color));
        }
        let c = ts(t.cx, t.cy);
        let r = t.r as f32 * zoom;
        let stroke = Stroke::new(1.4 * zoom, hex(flowmaid::journey::FACE_STROKE));
        painter.circle(c, r, hex(t.color), stroke);
        let ink = hex(flowmaid::journey::FACE_INK);
        for ex in [-0.32, 0.32] {
            painter.circle_filled(
                ts(t.cx + t.r * ex, t.cy - t.r * 0.18),
                (t.r * 0.1) as f32 * zoom,
                ink,
            );
        }
        // Mulut: kurva kuadratik, arah lengkung ikut skor.
        let my = t.cy + t.r * 0.24;
        let curv = (t.score as f64 - 3.0) * t.r * 0.28;
        painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [ts(t.cx - t.r * 0.42, my), ts(t.cx, my + curv), ts(t.cx + t.r * 0.42, my)],
            false,
            Color32::TRANSPARENT,
            Stroke::new(1.6 * zoom, ink),
        ));
        painter.text(
            ts(t.label_pos.0, t.label_pos.1),
            Align2::CENTER_CENTER,
            &t.name,
            zfont(13.0, zoom),
            TEXT,
        );
    }
    // Judul.
    if let Some(title) = &js.title {
        painter.text(
            ts(js.title_pos.0, js.title_pos.1),
            Align2::CENTER_CENTER,
            title,
            zfont(17.0, zoom),
            TEXT,
        );
    }
    // Legenda aktor.
    for it in &js.legend {
        painter.circle_filled(ts(it.x + 6.0, it.y), 6.0 * zoom, hex(it.color));
        painter.text(
            ts(it.x + 16.0, it.y),
            Align2::LEFT_CENTER,
            &it.name,
            zfont(13.0, zoom),
            TEXT,
        );
    }
}

/// Pie chart: judul, sektor, label persen, legenda. Cermin dari
/// `pie::to_svg`, memakai geometri `PieScene` yang sama. `labels` /
/// `empty` sudah di-precompute di `set_static_pie` (bebas alokasi).
pub(crate) fn draw_pie(
    p: &egui::Painter,
    ps: &PieScene,
    labels: &[Option<String>],
    empty: bool,
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    if let Some(t) = &ps.title {
        p.text(
            ts(ps.title_pos.0, ps.title_pos.1),
            Align2::CENTER_CENTER,
            t,
            zfont(16.0, zoom),
            TEXT,
        );
    }
    let center = ts(ps.cx, ps.cy);
    let r = ps.r as f32 * zoom;
    if empty {
        p.circle_stroke(center, r, Stroke::new(1.6 * zoom, EDGE));
    }
    for (i, sl) in ps.slices.iter().enumerate() {
        if sl.frac <= 0.0 {
            continue;
        }
        draw_wedge(p, center, r, sl.start_angle, sl.end_angle, accent_color(i), zoom);
    }
    for (sl, label) in ps.slices.iter().zip(labels) {
        let Some(text) = label else { continue };
        let mid = (sl.start_angle + sl.end_angle) / 2.0;
        let lx = ps.cx + ps.r * pie::LABEL_R * mid.sin();
        let ly = ps.cy - ps.r * pie::LABEL_R * mid.cos();
        p.text(
            ts(lx, ly),
            Align2::CENTER_CENTER,
            text,
            zfont(13.0, zoom),
            Color32::WHITE,
        );
    }
    for (i, row) in ps.legend.iter().enumerate() {
        let sw = pie::SWATCH as f32 * zoom;
        p.rect_filled(
            Rect::from_min_size(ts(row.x, row.y - pie::SWATCH / 2.0), Vec2::splat(sw)),
            2.0 * zoom,
            accent_color(i),
        );
        p.text(
            ts(row.x + pie::SWATCH + 8.0, row.y),
            Align2::LEFT_CENTER,
            &row.text,
            zfont(13.0, zoom),
            TEXT,
        );
    }
}

/// One pie sector, tessellated as a triangle fan from the centre
/// (valid for any sweep, unlike a single convex polygon), with the
/// full white outline (radial edges + arc rim) mirroring the SVG
/// slice stroke.
fn draw_wedge(p: &egui::Painter, center: Pos2, r: f32, a0: f64, a1: f64, color: Color32, zoom: f32) {
    let white = Stroke::new(1.5 * zoom, Color32::WHITE);
    let span = a1 - a0;
    if span >= std::f64::consts::TAU - 1e-6 {
        // Paritas SVG: <circle ... stroke="#ffffff" stroke-width="1.5">.
        p.circle(center, r, color, white);
        return;
    }
    let steps = ((span / 0.15).ceil() as usize).max(1);
    let pt = |a: f64| Pos2::new(center.x + r * a.sin() as f32, center.y - r * a.cos() as f32);
    let mut prev = pt(a0);
    for k in 1..=steps {
        let cur = pt(a0 + span * (k as f64 / steps as f64));
        p.add(egui::epaint::PathShape::convex_polygon(
            vec![center, prev, cur],
            color,
            Stroke::NONE,
        ));
        // Rim busur ikut di-stroke putih, seperti path SVG-nya.
        p.line_segment([prev, cur], white);
        prev = cur;
    }
    p.line_segment([center, pt(a0)], white);
    p.line_segment([center, pt(a1)], white);
}

/// Sequence diagram: frames, lifelines, activation bars, notes,
/// messages (with head glyphs), and participant boxes. Cermin dari
/// `seq::to_svg`, memakai geometri `SeqScene` yang sama.
pub(crate) fn draw_sequence(
    p: &egui::Painter,
    sc: &SeqScene,
    labels: &[String],
    ts: &impl Fn(f64, f64) -> Pos2,
    zoom: f32,
) {
    let guide = GUIDE;
    // Frame borders (background).
    for f in &sc.frames {
        p.rect_stroke(
            Rect::from_min_max(ts(f.x, f.y), ts(f.x + f.w, f.y + f.h)),
            4.0 * zoom,
            Stroke::new(1.2 * zoom, guide),
        );
    }
    // Lifelines (dashed) + activation bars.
    for l in &sc.lifelines {
        let s = Stroke::new(1.0 * zoom, guide);
        dashed_line(p, ts(l.x, l.y0), ts(l.x, l.y1), s, 4.0 * zoom, 4.0 * zoom);
    }
    for a in &sc.activations {
        p.rect(
            Rect::from_min_max(ts(a.x - 4.0, a.y0), ts(a.x + 4.0, a.y1)),
            0.0,
            Color32::WHITE,
            Stroke::new(1.4 * zoom, accent_color(a.participant)),
        );
    }
    // Frame chips, labels, and else/and dividers (over the lifelines).
    for f in &sc.frames {
        let kw = f.kind.keyword();
        let cw = flowmaid::layout::text_width(kw) + 14.0;
        p.rect(
            Rect::from_min_max(ts(f.x, f.y), ts(f.x + cw, f.y + 18.0)),
            0.0,
            CHIP_FILL,
            Stroke::new(1.0 * zoom, guide),
        );
        p.text(
            ts(f.x + cw / 2.0, f.y + 9.0),
            Align2::CENTER_CENTER,
            kw,
            zfont(13.0, zoom),
            TEXT,
        );
        if !f.label.is_empty() {
            p.text(
                ts(f.x + cw + 6.0, f.y + 9.0),
                Align2::LEFT_CENTER,
                format!("[{}]", f.label),
                zfont(13.0, zoom),
                TEXT,
            );
        }
        for (dy, dl) in &f.dividers {
            let s = Stroke::new(1.0 * zoom, guide);
            dashed_line(p, ts(f.x, *dy), ts(f.x + f.w, *dy), s, 5.0 * zoom, 4.0 * zoom);
            if !dl.is_empty() {
                p.text(
                    ts(f.x + f.w / 2.0, dy + 12.0),
                    Align2::CENTER_CENTER,
                    format!("[{}]", dl),
                    zfont(13.0, zoom),
                    TEXT,
                );
            }
        }
    }
    // Notes.
    for nb in &sc.notes {
        p.rect(
            Rect::from_min_max(ts(nb.x, nb.y), ts(nb.x + nb.w, nb.y + nb.h)),
            3.0 * zoom,
            NOTE_FILL,
            Stroke::new(1.2 * zoom, NOTE_STROKE),
        );
        p.text(
            ts(nb.x + nb.w / 2.0, nb.y + nb.h / 2.0),
            Align2::CENTER_CENTER,
            &nb.text,
            zfont(13.0, zoom),
            TEXT,
        );
    }
    // Messages: polyline + head glyph + label (with autonumber).
    // `labels` sudah di-precompute di set_static_seq (tanpa format!
    // per frame); indeks sejajar dengan sc.messages.
    for (i, m) in sc.messages.iter().enumerate() {
        let stroke = Stroke::new(1.6 * zoom, EDGE);
        for w in m.points.windows(2) {
            let (a, b) = (ts(w[0].0, w[0].1), ts(w[1].0, w[1].1));
            if m.dashed {
                dashed_line(p, a, b, stroke, 6.0 * zoom, 4.0 * zoom);
            } else {
                p.line_segment([a, b], stroke);
            }
        }
        let np = m.points.len();
        draw_seq_head(p, &seq::head(m.points[np - 1], m.points[np - 2], m.head), ts, zoom);
        if m.text.is_empty() && m.number.is_none() {
            continue;
        }
        let anchor = if m.label_centered {
            Align2::CENTER_CENTER
        } else {
            Align2::LEFT_CENTER
        };
        let label = labels.get(i).map(String::as_str).unwrap_or(&m.text);
        p.text(
            ts(m.label_pos.0, m.label_pos.1),
            anchor,
            label,
            zfont(13.0, zoom),
            TEXT,
        );
    }
    // Participant boxes last (crisp over the lifeline tops).
    for (i, b) in sc.boxes.iter().enumerate() {
        let accent = accent_color(i);
        let (fill, text_fill) = if b.actor {
            (Color32::WHITE, accent)
        } else {
            (accent, Color32::WHITE)
        };
        p.rect(
            Rect::from_min_max(ts(b.x, b.y), ts(b.x + b.w, b.y + b.h)),
            4.0 * zoom,
            fill,
            Stroke::new(1.6 * zoom, accent),
        );
        p.text(
            ts(b.x + b.w / 2.0, b.y + b.h / 2.0),
            Align2::CENTER_CENTER,
            &b.label,
            zfont(13.5, zoom),
            text_fill,
        );
    }
}

/// Filled-triangle / open head for a sequence message (plain
/// geometry from `seq::head`).
fn draw_seq_head(p: &egui::Painter, h: &seq::Head, ts: &impl Fn(f64, f64) -> Pos2, zoom: f32) {
    if !h.polygon.is_empty() {
        let pts: Vec<Pos2> = h.polygon.iter().map(|(x, y)| ts(*x, *y)).collect();
        p.add(egui::epaint::PathShape::convex_polygon(pts, EDGE, Stroke::NONE));
    }
    for [a, b] in &h.segments {
        p.line_segment([ts(a.0, a.1), ts(b.0, b.1)], Stroke::new(1.6 * zoom, EDGE));
    }
}
