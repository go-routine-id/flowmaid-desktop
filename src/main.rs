//! flowmaid desktop — editor diagram interaktif di atas engine flowmaid.
//!
//! - Panel kiri : editor teks Mermaid, live dengan pola "last good render"
//! - Panel kanan: kanvas — node bisa DIGESER, edge mengikuti realtime
//! - Zoom (pinch / ctrl+scroll / tombol ±) dan pan (scroll / drag kanvas)
//! - Mendukung flowchart DAN erDiagram (tabel entitas + crow's foot,
//!   sama-sama bisa digeser)
//! - Drag & drop file .mmd ke jendela untuk membukanya
//! - "Ekspor SVG" menyimpan susunan saat ini (termasuk hasil geseran)
//!
//! Jalankan: `cargo run --release` (engine `flowmaid` ditarik
//! langsung dari crates.io).

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use flowmaid::er::{self, ErTable};
use flowmaid::model::{Card, EdgeKind, ErDiagram, Graph, Shape};
use flowmaid::scene::{route, scene, to_svg, Scene, SceneNode};
use flowmaid::Document;
use std::collections::HashMap;

const EDGE: Color32 = Color32::from_rgb(0x44, 0x50, 0x7a);
const FILL: Color32 = Color32::from_rgb(0xee, 0xf1, 0xfb);
const BORDER: Color32 = Color32::from_rgb(0x5b, 0x6d, 0xc0);
const TEXT: Color32 = Color32::from_rgb(0x23, 0x28, 0x40);
const LABEL_BORDER: Color32 = Color32::from_rgb(0xd5, 0xd9, 0xec);
const TYPE_MUTED: Color32 = Color32::from_rgb(0x6a, 0x70, 0x86);

const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 4.0;

const CONTOH: &str = "%% Geser node dengan mouse, atau edit teks ini.\nflowchart TD\n    A([Mulai]) --> B[Baca input]\n    B --> C{Valid?}\n    C -->|ya| D[Proses data]\n    C -->|tidak| E[Tampilkan error]\n    E --> B\n    D ==> F((Selesai))\n";

fn main() -> eframe::Result<()> {
    let src = std::env::args()
        .nth(1)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| CONTOH.to_string());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1150.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "flowmaid desktop",
        opts,
        Box::new(|_cc| Ok(Box::new(App::new(src)))),
    )
}

/// Dokumen valid terakhir — flowchart atau diagram ER.
enum Model {
    Flow(Graph),
    Er(ErDiagram),
}

impl Model {
    /// Kunci identitas node/entitas ke-i, untuk mempertahankan
    /// posisi geseran saat teks diedit.
    fn keys(&self) -> Vec<&str> {
        match self {
            Model::Flow(g) => g.nodes.iter().map(|n| n.id.as_str()).collect(),
            Model::Er(d) => d.entities.iter().map(|e| e.name.as_str()).collect(),
        }
    }
}

struct App {
    src: String,
    model: Model, // dokumen valid terakhir
    pos: Vec<(f64, f64)>,        // posisi node/entitas, milik aplikasi (bisa digeser)
    scn: Scene,                  // geometri terkini untuk digambar
    tables: Vec<ErTable>,        // data tabel ER (kosong untuk flowchart)
    cards: Vec<(Card, Card)>,    // kardinalitas per relasi ER, sejajar scn.edges
    error: Option<String>,
    status: String,
    zoom: f32,         // faktor zoom kanvas (1.0 = 100%)
    pan: Vec2,         // geseran kanvas, piksel layar
    canvas_size: Vec2, // ukuran kanvas frame terakhir (jangkar zoom via tombol)
}

impl App {
    fn new(src: String) -> Self {
        let mut app = App {
            src,
            model: Model::Flow(Graph::default()),
            pos: Vec::new(),
            scn: Scene {
                nodes: Vec::new(),
                edges: Vec::new(),
                width: 0.0,
                height: 0.0,
            },
            tables: Vec::new(),
            cards: Vec::new(),
            error: None,
            status: "geser node dengan mouse".into(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            canvas_size: Vec2::ZERO,
        };
        app.reparse();
        app
    }

    /// Parse ulang teks. Bila gagal, pertahankan render valid terakhir
    /// (pola "last good render"). Posisi node/entitas yang kuncinya
    /// masih ada dipertahankan supaya geseran tidak hilang saat mengetik.
    fn reparse(&mut self) {
        match flowmaid::parser::parse_document(&self.src) {
            Ok(doc) => {
                let old: HashMap<String, (f64, f64)> = self
                    .model
                    .keys()
                    .iter()
                    .zip(&self.pos)
                    .map(|(k, p)| (k.to_string(), *p))
                    .collect();
                match doc {
                    Document::Flowchart(g) => {
                        let auto = scene(&g);
                        self.pos = g
                            .nodes
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                *old.get(n.id.as_str())
                                    .unwrap_or(&(auto.nodes[i].x, auto.nodes[i].y))
                            })
                            .collect();
                        self.model = Model::Flow(g);
                    }
                    Document::Er(d) => {
                        let auto = er::scene(&d);
                        self.pos = d
                            .entities
                            .iter()
                            .enumerate()
                            .map(|(i, e)| {
                                *old.get(e.name.as_str()).unwrap_or(&(
                                    auto.scene.nodes[i].x,
                                    auto.scene.nodes[i].y,
                                ))
                            })
                            .collect();
                        self.model = Model::Er(d);
                    }
                }
                self.reroute();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn reroute(&mut self) {
        match &self.model {
            Model::Flow(g) => {
                self.scn = route(g, &self.pos);
                self.tables.clear();
                self.cards.clear();
            }
            Model::Er(d) => {
                let es = er::route(d, &self.pos);
                self.scn = es.scene;
                self.tables = es.tables;
                self.cards = es.cards;
            }
        }
    }

    /// Kembali ke tata letak otomatis engine.
    fn autolayout(&mut self) {
        match &self.model {
            Model::Flow(g) => {
                self.scn = scene(g);
                self.tables.clear();
                self.cards.clear();
            }
            Model::Er(d) => {
                let es = er::scene(d);
                self.scn = es.scene;
                self.tables = es.tables;
                self.cards = es.cards;
            }
        }
        self.pos = self.scn.nodes.iter().map(|n| (n.x, n.y)).collect();
        self.reset_view();
    }

    /// SVG dari susunan saat ini (termasuk hasil geseran).
    fn export_svg(&self) -> String {
        match &self.model {
            Model::Flow(_) => to_svg(&self.scn),
            Model::Er(d) => er::to_svg(&er::route(d, &self.pos)),
        }
    }

    /// Zoom dengan jangkar tetap (koordinat lokal kanvas): titik dunia
    /// yang berada di bawah jangkar tidak bergeser di layar.
    fn zoom_around(&mut self, factor: f32, anchor: Vec2) {
        let target = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = target / self.zoom;
        self.pan = anchor - (anchor - self.pan) * k;
        self.zoom = target;
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drag & drop FILE .mmd ke jendela.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.first() {
            if let Some(p) = &f.path {
                match std::fs::read_to_string(p) {
                    Ok(t) => {
                        self.src = t;
                        self.reparse();
                        self.reset_view();
                        self.status = format!("dibuka: {}", p.display());
                    }
                    Err(e) => self.status = format!("gagal membuka: {}", e),
                }
            }
        }

        egui::SidePanel::left("editor")
            .default_width(330.0)
            .show(ctx, |ui| {
                ui.heading("flowmaid");
                ui.label("Geser node di kanan, edit teks di bawah,\natau drop file .mmd ke jendela ini.");
                ui.horizontal(|ui| {
                    if ui.button("Tata ulang otomatis").clicked() {
                        self.autolayout();
                    }
                    if ui.button("Ekspor SVG").clicked() {
                        match std::fs::write("flowmaid-export.svg", self.export_svg()) {
                            Ok(_) => self.status = "tersimpan: flowmaid-export.svg".into(),
                            Err(e) => self.status = format!("gagal menyimpan: {}", e),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("−").clicked() {
                        self.zoom_around(1.0 / 1.25, self.canvas_size / 2.0);
                    }
                    if ui
                        .button(format!("{:.0}%", self.zoom * 100.0))
                        .on_hover_text("klik untuk reset tampilan")
                        .clicked()
                    {
                        self.reset_view();
                    }
                    if ui.button("+").clicked() {
                        self.zoom_around(1.25, self.canvas_size / 2.0);
                    }
                    ui.small("pinch/ctrl+scroll = zoom\nscroll / drag kanvas = geser");
                });
                match &self.error {
                    Some(e) => {
                        ui.colored_label(Color32::from_rgb(200, 60, 60), format!("parse: {}", e));
                    }
                    None => {
                        ui.label(&self.status);
                    }
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut self.src)
                            .code_editor()
                            .desired_rows(30)
                            .desired_width(f32::INFINITY),
                    );
                    if r.changed() {
                        self.reparse();
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let canvas = ui.max_rect();
            self.canvas_size = canvas.size();

            // 0) Input kanvas: drag area kosong / scroll = pan,
            //    pinch / ctrl+scroll = zoom berjangkar di kursor.
            //    Didaftarkan SEBELUM node agar node menang saat tumpang tindih.
            let bg = ui.interact(canvas, egui::Id::new("flowrs-canvas"), Sense::drag());
            if bg.dragged() {
                self.pan += bg.drag_delta();
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
            }
            if ui.rect_contains_pointer(canvas) {
                let (zd, scroll, mouse) =
                    ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
                self.pan += scroll;
                if zd != 1.0 {
                    let anchor = mouse.map_or(self.canvas_size / 2.0, |m| m - canvas.min);
                    self.zoom_around(zd, anchor);
                }
            }
            let (zoom, pan) = (self.zoom, self.pan);
            // Koordinat dunia (scene) -> layar.
            let ts = |x: f64, y: f64| canvas.min + pan + Vec2::new(x as f32, y as f32) * zoom;

            // 1) Interaksi drag NODE dulu (rect dalam koordinat layar).
            let rects: Vec<Rect> = self
                .scn
                .nodes
                .iter()
                .map(|n| {
                    Rect::from_center_size(ts(n.x, n.y), Vec2::new(n.w as f32, n.h as f32) * zoom)
                })
                .collect();
            let mut moved = false;
            for (i, rect) in rects.iter().enumerate() {
                let resp = ui.interact(*rect, egui::Id::new(("flowrs-node", i)), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                }
                if resp.dragged() {
                    let d = resp.drag_delta() / zoom; // delta layar -> dunia
                    self.pos[i].0 += d.x as f64;
                    self.pos[i].1 += d.y as f64;
                    moved = true;
                }
            }
            if moved {
                self.reroute();
            }

            // 2) Gambar edge lalu node/tabel dari geometri terbaru.
            let painter = ui.painter();
            let is_er = !self.tables.is_empty();
            for (i, e) in self.scn.edges.iter().enumerate() {
                let p = e.bezier.map(|(x, y)| ts(x, y));
                let sw = (if matches!(e.kind, EdgeKind::Thick) { 3.4 } else { 1.7 }) * zoom;
                let stroke = Stroke::new(sw, EDGE);
                if matches!(e.kind, EdgeKind::Dotted) {
                    dashed_bezier(painter, p, stroke);
                } else {
                    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                        p,
                        false,
                        Color32::TRANSPARENT,
                        stroke,
                    ));
                }
                if is_er {
                    // Notasi crow's foot di kedua ujung relasi.
                    let (cf, ct) = self.cards[i];
                    draw_glyph(painter, &er::glyph(e.bezier[0], e.bezier[1], cf), &ts, zoom);
                    draw_glyph(painter, &er::glyph(e.bezier[3], e.bezier[2], ct), &ts, zoom);
                } else if !matches!(e.kind, EdgeKind::Open) {
                    arrow_head(painter, p, EDGE, zoom);
                }
                if let Some((t, (lx, ly), lw)) = &e.label {
                    let c = ts(*lx, *ly);
                    let r = Rect::from_center_size(c, Vec2::new(*lw as f32, 20.0) * zoom);
                    painter.rect(r, 4.0 * zoom, Color32::WHITE, Stroke::new(1.0 * zoom, LABEL_BORDER));
                    painter.text(c, Align2::CENTER_CENTER, t, FontId::proportional(13.0 * zoom), TEXT);
                }
            }
            if is_er {
                for (n, t) in self.scn.nodes.iter().zip(&self.tables) {
                    draw_table(painter, n, t, ts(n.x, n.y), zoom);
                }
            } else {
                for n in &self.scn.nodes {
                    draw_node(painter, n, ts(n.x, n.y), zoom);
                }
            }
        });
    }
}

fn draw_node(p: &egui::Painter, n: &SceneNode, c: Pos2, zoom: f32) {
    let (w, h) = (n.w as f32 * zoom, n.h as f32 * zoom);
    let stroke = Stroke::new(1.6 * zoom, BORDER);
    match n.shape {
        Shape::Circle => {
            p.circle(c, w / 2.0, FILL, stroke);
        }
        Shape::Diamond => {
            let pts = vec![
                Pos2::new(c.x, c.y - h / 2.0),
                Pos2::new(c.x + w / 2.0, c.y),
                Pos2::new(c.x, c.y + h / 2.0),
                Pos2::new(c.x - w / 2.0, c.y),
            ];
            p.add(egui::epaint::PathShape::convex_polygon(pts, FILL, stroke));
        }
        _ => {
            let r = Rect::from_center_size(c, Vec2::new(w, h));
            let round = match n.shape {
                Shape::Rounded => 9.0 * zoom,
                Shape::Stadium => h / 2.0,
                _ => 3.0 * zoom,
            };
            p.rect(r, round, FILL, stroke);
        }
    }
    p.text(c, Align2::CENTER_CENTER, &n.label, FontId::proportional(14.0 * zoom), TEXT);
}

/// Tabel entitas ER: header berwarna + baris atribut
/// (tipe redup | nama | tag kunci rata kanan).
fn draw_table(p: &egui::Painter, n: &SceneNode, t: &ErTable, c: Pos2, zoom: f32) {
    use flowmaid::er::{COL_GAP, HEADER_H, PAD, ROW_H};
    let (w, h) = (n.w as f32 * zoom, n.h as f32 * zoom);
    let x0 = c.x - w / 2.0;
    let y0 = c.y - h / 2.0;
    let round = 4.0 * zoom;
    p.rect(
        Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(w, h)),
        round,
        Color32::WHITE,
        Stroke::new(1.6 * zoom, BORDER),
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
        BORDER,
        Stroke::NONE,
    );
    p.text(
        Pos2::new(c.x, y0 + hh / 2.0),
        Align2::CENTER_CENTER,
        &t.name,
        FontId::proportional(13.5 * zoom),
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
        let f = FontId::proportional(12.5 * zoom);
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

/// Kepala panah di ujung bezier, searah turunan kurva di t=1.
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
            u * u * u * b[0].x + 3.0 * u * u * t * b[1].x + 3.0 * u * t * t * b[2].x + t * t * t * b[3].x,
            u * u * u * b[0].y + 3.0 * u * u * t * b[1].y + 3.0 * u * t * t * b[2].y + t * t * t * b[3].y,
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
