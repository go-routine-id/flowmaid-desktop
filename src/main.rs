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
use std::path::{Path, PathBuf};

const EDGE: Color32 = Color32::from_rgb(0x44, 0x50, 0x7a);
const TEXT: Color32 = Color32::from_rgb(0x23, 0x28, 0x40);
const LABEL_BORDER: Color32 = Color32::from_rgb(0xd5, 0xd9, 0xec);
const TYPE_MUTED: Color32 = Color32::from_rgb(0x6a, 0x70, 0x86);

/// `#rrggbb` (tema engine) → Color32, supaya kanvas dan ekspor SVG
/// memakai warna yang persis sama.
fn hex(c: &str) -> Color32 {
    if c.len() == 7 && c.starts_with('#') {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&c[1..3], 16),
            u8::from_str_radix(&c[3..5], 16),
            u8::from_str_radix(&c[5..7], 16),
        ) {
            return Color32::from_rgb(r, g, b);
        }
    }
    Color32::GRAY
}

const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 4.0;

const CONTOH: &str = "%% Geser node dengan mouse, atau edit teks ini.\n%% Warna kustom: style / classDef / ::: ala mermaid.\nflowchart TD\n    A([Mulai]) --> B[Baca input]\n    B --> C{Valid?}\n    C -->|ya| D[Proses data]\n    C -->|tidak| E[Tampilkan error]\n    E --> B\n    D ==> F((Selesai))\n    classDef bahaya fill:#ffe3e3,stroke:#e03131,color:#c92a2a\n    E:::bahaya\n";

fn main() -> eframe::Result<()> {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let (src, path) = match arg {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(t) => (t, Some(p)),
            Err(_) => (CONTOH.to_string(), None),
        },
        None => (CONTOH.to_string(), None),
    };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1150.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "flowmaid desktop",
        opts,
        Box::new(move |cc| {
            let recent: Vec<String> = cc
                .storage
                .and_then(|s| s.get_string("recent"))
                .map(|s| s.lines().filter(|l| !l.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();
            Ok(Box::new(App::new(src, path, recent)))
        }),
    )
}

/// Aksi yang bisa membuang perubahan; ditunda ke dialog konfirmasi
/// bila dokumen sedang dirty.
enum Pending {
    New,
    OpenDialog,
    OpenPath(PathBuf),
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
    path: Option<PathBuf>, // file yang sedang dibuka (None = belum disimpan)
    saved_src: String,     // isi terakhir yang tersimpan, untuk deteksi dirty
    recent: Vec<String>,   // file terakhir dibuka, terbaru di depan
    pending: Option<Pending>, // aksi menunggu konfirmasi buang-perubahan
    last_title: String,
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
    fn new(src: String, path: Option<PathBuf>, recent: Vec<String>) -> Self {
        let saved_src = src.clone();
        let mut app = App {
            src,
            path,
            saved_src,
            recent,
            pending: None,
            last_title: String::new(),
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

    fn dirty(&self) -> bool {
        self.src != self.saved_src
    }

    fn push_recent(&mut self, p: &Path) {
        let s = p.display().to_string();
        self.recent.retain(|r| r != &s);
        self.recent.insert(0, s);
        self.recent.truncate(8);
    }

    fn open_path(&mut self, p: PathBuf) {
        match std::fs::read_to_string(&p) {
            Ok(t) => {
                self.src = t;
                self.saved_src = self.src.clone();
                self.reparse();
                self.reset_view();
                self.status = format!("dibuka: {}", p.display());
                self.push_recent(&p);
                self.path = Some(p);
            }
            Err(e) => self.status = format!("gagal membuka: {}", e),
        }
    }

    fn new_file(&mut self) {
        self.src = CONTOH.to_string();
        self.saved_src = self.src.clone();
        self.path = None;
        self.reparse();
        self.reset_view();
        self.status = "dokumen baru".into();
    }

    /// Simpan ke file saat ini; belum punya file → Simpan Sebagai.
    fn save_doc(&mut self) {
        match self.path.clone() {
            Some(p) => self.write_to(&p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let mut dlg = rfd::FileDialog::new().add_filter("Mermaid", &["mmd"]);
        match &self.path {
            Some(p) => {
                if let Some(dir) = p.parent() {
                    dlg = dlg.set_directory(dir);
                }
                if let Some(n) = p.file_name() {
                    dlg = dlg.set_file_name(n.to_string_lossy());
                }
            }
            None => dlg = dlg.set_file_name("diagram.mmd"),
        }
        if let Some(p) = dlg.save_file() {
            self.write_to(&p);
            self.push_recent(&p);
            self.path = Some(p);
        }
    }

    fn write_to(&mut self, p: &Path) {
        match std::fs::write(p, &self.src) {
            Ok(_) => {
                self.saved_src = self.src.clone();
                self.status = format!("tersimpan: {}", p.display());
            }
            Err(e) => self.status = format!("gagal menyimpan: {}", e),
        }
    }

    fn export_svg_file(&mut self) {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}.svg", s.to_string_lossy()))
            .unwrap_or_else(|| "diagram.svg".into());
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("SVG", &["svg"])
            .set_file_name(name)
            .save_file()
        {
            match std::fs::write(&p, self.export_svg()) {
                Ok(_) => self.status = format!("tersimpan: {}", p.display()),
                Err(e) => self.status = format!("gagal menyimpan: {}", e),
            }
        }
    }

    /// Jalankan aksi yang bisa membuang perubahan; kalau dokumen
    /// dirty, tahan dulu di dialog konfirmasi.
    fn request(&mut self, act: Pending) {
        if self.dirty() {
            self.pending = Some(act);
        } else {
            self.perform(act);
        }
    }

    fn perform(&mut self, act: Pending) {
        match act {
            Pending::New => self.new_file(),
            Pending::OpenDialog => {
                let mut dlg = rfd::FileDialog::new().add_filter("Mermaid", &["mmd", "txt"]);
                if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
                    dlg = dlg.set_directory(dir);
                }
                if let Some(p) = dlg.pick_file() {
                    self.open_path(p);
                }
            }
            Pending::OpenPath(p) => self.open_path(p),
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("recent", self.recent.join("\n"));
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Judul jendela: "• nama.mmd — flowmaid desktop" saat dirty.
        let title = format!(
            "{}{} — flowmaid desktop",
            if self.dirty() { "• " } else { "" },
            self.path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "tanpa judul".into())
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        // Shortcut file. Simpan-Sebagai dicek sebelum Simpan supaya
        // ⇧⌘S tidak termakan ⌘S.
        use egui::{Key, KeyboardShortcut, Modifiers};
        const SAVE_AS: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
        const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE_AS)) {
            self.save_as();
        } else if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.save_doc();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&OPEN)) {
            self.request(Pending::OpenDialog);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&NEW)) {
            self.request(Pending::New);
        }

        // Drag & drop FILE .mmd ke jendela.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(p) = dropped.first().and_then(|f| f.path.clone()) {
            self.request(Pending::OpenPath(p));
        }

        // Dialog konfirmasi untuk aksi yang membuang perubahan.
        if self.pending.is_some() {
            let mut decided: Option<Option<Pending>> = None;
            egui::Window::new("Perubahan belum disimpan")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Dokumen ini punya perubahan yang belum disimpan.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Simpan dulu").clicked() {
                            self.save_doc();
                            // Simpan bisa dibatalkan dari dialog Simpan
                            // Sebagai — lanjut hanya bila benar tersimpan.
                            decided = if self.dirty() {
                                Some(None)
                            } else {
                                Some(self.pending.take())
                            };
                        }
                        if ui.button("Buang perubahan").clicked() {
                            decided = Some(self.pending.take());
                        }
                        if ui.button("Batal").clicked() {
                            decided = Some(None);
                        }
                    });
                });
            match decided {
                Some(Some(act)) => self.perform(act),
                Some(None) => self.pending = None,
                None => {}
            }
        }

        // Menu bar.
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Baru        ⌘N").clicked() {
                        self.request(Pending::New);
                        ui.close_menu();
                    }
                    if ui.button("Buka…       ⌘O").clicked() {
                        self.request(Pending::OpenDialog);
                        ui.close_menu();
                    }
                    ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
                        ui.menu_button("Baru dibuka", |ui| {
                            for r in self.recent.clone() {
                                if ui.button(&r).clicked() {
                                    self.request(Pending::OpenPath(PathBuf::from(&r)));
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                    ui.separator();
                    if ui.button("Simpan      ⌘S").clicked() {
                        self.save_doc();
                        ui.close_menu();
                    }
                    if ui.button("Simpan Sebagai… ⇧⌘S").clicked() {
                        self.save_as();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Ekspor SVG…").clicked() {
                        self.export_svg_file();
                        ui.close_menu();
                    }
                });
            });
        });

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
                        self.export_svg_file();
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
            let mut hovered_node: Option<usize> = None;
            for (i, rect) in rects.iter().enumerate() {
                let resp = ui.interact(*rect, egui::Id::new(("flowrs-node", i)), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                    hovered_node = Some(i);
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
                for (i, (n, t)) in self.scn.nodes.iter().zip(&self.tables).enumerate() {
                    let accent = hex(flowmaid::style::accent(i));
                    draw_table(painter, n, t, ts(n.x, n.y), zoom, accent, hovered_node == Some(i));
                }
            } else {
                for (i, n) in self.scn.nodes.iter().enumerate() {
                    draw_node(painter, n, ts(n.x, n.y), zoom, hovered_node == Some(i));
                }
            }
        });
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
    match n.shape {
        Shape::Circle => {
            p.circle(c, w / 2.0, fill, stroke);
        }
        Shape::Diamond => {
            let pts = vec![
                Pos2::new(c.x, c.y - h / 2.0),
                Pos2::new(c.x + w / 2.0, c.y),
                Pos2::new(c.x, c.y + h / 2.0),
                Pos2::new(c.x - w / 2.0, c.y),
            ];
            p.add(egui::epaint::PathShape::convex_polygon(pts, fill, stroke));
        }
        _ => {
            let r = Rect::from_center_size(c, Vec2::new(w, h));
            let round = match n.shape {
                Shape::Rounded => 9.0 * zoom,
                Shape::Stadium => h / 2.0,
                _ => 3.0 * zoom,
            };
            p.rect(r, round, fill, stroke);
        }
    }
    p.text(
        c,
        Align2::CENTER_CENTER,
        &n.label,
        FontId::proportional(14.0 * zoom),
        text_color,
    );
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
