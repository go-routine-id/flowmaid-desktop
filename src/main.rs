//! flowmaid desktop — editor diagram interaktif di atas engine flowmaid.
//!
//! - Multi-file : tab dokumen ala editor — klik file membuka tab baru,
//!   Cmd+W menutup, tab dirty ditahan dialog, sesi tab dipulihkan
//! - Markdown   : buka .md → tiap blok ```mermaid jadi tab; Simpan
//!   menulis balik ke dalam fence-nya (crate `markdown`, via AST)
//! - Panel kiri : explorer folder ala VSCode (klik file .mmd untuk buka)
//! - Area utama : tab Preview | Split | Code
//!     - Preview: kanvas penuh — node bisa DIGESER, edge realtime,
//!       zoom (pinch / ctrl+scroll / tombol ±) dan pan (scroll / drag)
//!     - Split  : kanvas + editor teks berdampingan (default)
//!     - Code   : editor teks Mermaid penuh, pola "last good render"
//! - Mendukung flowchart, erDiagram (tabel entitas + crow's foot),
//!   classDiagram (box tiga kompartemen + glyph relasi UML), pie
//!   (sektor + legenda), dan sequenceDiagram (lifeline, pesan,
//!   activation, frame) — dua terakhir statis (tanpa geser)
//! - Drag & drop file .mmd ke jendela untuk membukanya; Ekspor SVG
//!
//! Jalankan: `cargo run --release` (engine `flowmaid` ditarik
//! langsung dari crates.io).

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use flowmaid::class::{self, ClassBox, RelStyle};
use flowmaid::er::{self, ErTable};
use flowmaid::model::{
    Card, ClassDiagram, EdgeKind, ErDiagram, Graph, PieChart, SequenceDiagram, Shape,
};
use flowmaid::pie::{self, PieScene};
use flowmaid::scene::{route, scene, to_svg, Scene, SceneNode};
use flowmaid::seq::{self, SeqScene};
use flowmaid::Document;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const EDGE: Color32 = Color32::from_rgb(0x44, 0x50, 0x7a);
const TEXT: Color32 = Color32::from_rgb(0x23, 0x28, 0x40);
const LABEL_BORDER: Color32 = Color32::from_rgb(0xd5, 0xd9, 0xec);
const TYPE_MUTED: Color32 = Color32::from_rgb(0x6a, 0x70, 0x86);
// Warna tetap jalur gambar sequence — const, bukan hex() per frame.
const GUIDE: Color32 = Color32::from_rgb(0xae, 0xb6, 0xd8);
const CHIP_FILL: Color32 = Color32::from_rgb(0xee, 0xf1, 0xfb);
const NOTE_FILL: Color32 = Color32::from_rgb(0xfc, 0xf2, 0xda);
const NOTE_STROKE: Color32 = Color32::from_rgb(0xd9, 0x91, 0x14);

/// Warna CSS (tema engine / style user) → Color32, supaya kanvas
/// dan ekspor SVG memakai warna yang persis sama. Mendukung
/// `#rrggbb`, shorthand `#rgb`, dan nama warna CSS yang umum —
/// semuanya bentuk yang diterima renderer SVG.
fn hex(c: &str) -> Color32 {
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
fn zfont(base: f32, zoom: f32) -> FontId {
    FontId::proportional(((base * zoom).max(2.0) * 2.0).round() / 2.0)
}

/// Warna accent engine di-parse sekali, bukan `hex()` (parsing string
/// CSS) per elemen per frame.
fn accent_color(i: usize) -> Color32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<Color32>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        (0..flowmaid::style::ACCENTS.len())
            .map(|k| hex(flowmaid::style::accent(k)))
            .collect()
    });
    t[i % t.len()]
}

const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 4.0;

const CONTOH: &str = "%% Geser node dengan mouse, atau edit teks ini.\n%% Warna kustom: style / classDef / ::: ala mermaid.\nflowchart TD\n    A([Mulai]) --> B[Baca input]\n    B --> C{Valid?}\n    C -->|ya| D[Proses data]\n    C -->|tidak| E[Tampilkan error]\n    E --> B\n    D ==> F((Selesai))\n    classDef bahaya fill:#ffe3e3,stroke:#e03131,color:#c92a2a\n    E:::bahaya\n";

fn main() -> eframe::Result<()> {
    let arg = std::env::args().nth(1).map(PathBuf::from);
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
                .map(|s| {
                    s.lines()
                        .filter(|l| !l.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let workspace = cc
                .storage
                .and_then(|s| s.get_string("workspace"))
                .map(PathBuf::from)
                .filter(|p| p.is_dir());
            // Tab sesi sebelumnya (hanya yang filenya masih ada).
            let tabs: Vec<PathBuf> = cc
                .storage
                .and_then(|s| s.get_string("tabs"))
                .map(|s| {
                    s.lines()
                        .filter(|l| !l.is_empty())
                        .map(PathBuf::from)
                        .filter(|p| p.is_file())
                        .collect()
                })
                .unwrap_or_default();
            // SEMUA pembukaan file — termasuk argumen CLI — lewat
            // open_path, supaya routing .md → blok mermaid dan
            // dedupe tab berlaku seragam. (Dulu argumen CLI dibaca
            // mentah ke editor: file .md gagal parse → preview kosong.)
            let mut app = App::new(CONTOH.to_string(), None, recent, workspace);
            for p in tabs {
                app.open_path(p);
            }
            match arg {
                // File dari CLI dibuka terakhir → jadi tab aktif.
                Some(p) => app.open_path(p),
                None => app.switch_to(0),
            }
            Ok(Box::new(app))
        }),
    )
}

/// Aksi yang bisa membuang perubahan; ditunda ke dialog konfirmasi
/// bila dokumen yang bersangkutan sedang dirty. Sejak ada tab,
/// membuka file tak pernah membuang apa pun (selalu jadi tab baru) —
/// hanya menutup tab yang butuh konfirmasi.
enum Pending {
    CloseTab(usize),
}

/// Tab area utama: pratinjau, terbelah, atau editor teks.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Preview,
    Split,
    Code,
}

/// Dokumen valid terakhir. Flow/ER/class punya node yang bisa
/// digeser; pie & sequence statis (digambar apa adanya).
enum Model {
    Flow(Graph),
    Er(ErDiagram),
    Class(ClassDiagram),
    Pie(PieChart),
    Sequence(SequenceDiagram),
}

impl Model {
    /// Kunci identitas node/entitas/class ke-i, untuk mempertahankan
    /// posisi geseran saat teks diedit. Pie/sequence tak punya node
    /// yang bisa digeser, jadi kosong.
    fn keys(&self) -> Vec<&str> {
        match self {
            Model::Flow(g) => g.nodes.iter().map(|n| n.id.as_str()).collect(),
            Model::Er(d) => d.entities.iter().map(|e| e.name.as_str()).collect(),
            Model::Class(d) => d.classes.iter().map(|c| c.name.as_str()).collect(),
            Model::Pie(_) | Model::Sequence(_) => Vec::new(),
        }
    }
}

/// Satu entri hasil listing folder. Nama tampilan & jenis (folder /
/// file) di-precompute saat cache diisi, supaya draw_tree tidak
/// melakukan syscall stat() maupun alokasi String per entri per frame.
struct TreeEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

/// Dokumen ini berasal dari satu blok ```mermaid di dalam file
/// Markdown: `path` = file .md induk, `index` = blok mermaid ke-n
/// (0-based). Menyimpan berarti menulis balik KE DALAM fence-nya.
#[derive(Clone)]
struct MdHost {
    path: PathBuf,
    index: usize,
}

/// State lengkap satu dokumen (satu tab). Dokumen AKTIF tinggal di
/// field-field `App` (kode gambar/editor tak perlu berubah); struct
/// ini memarkir tab non-aktif, di-swap saat pindah tab. Entri milik
/// tab aktif di `App::docs` adalah cangkang kosong.
struct Doc {
    src: String,
    path: Option<PathBuf>,
    md_host: Option<MdHost>,
    saved_src: String,
    model: Model,
    pos: Vec<(f64, f64)>,
    scn: Scene,
    tables: Vec<ErTable>,
    cards: Vec<(Card, Card)>,
    boxes: Vec<ClassBox>,
    rels: Vec<RelStyle>,
    pie: Option<PieScene>,
    seq: Option<SeqScene>,
    pie_labels: Vec<Option<String>>,
    pie_empty: bool,
    seq_labels: Vec<String>,
    /// Some = tab ini DOKUMEN Markdown ter-render (bukan diagram).
    mdoc: Option<MdDoc>,
    error: Option<String>,
    zoom: f32,
    pan: Vec2,
}

impl Doc {
    /// Cangkang kosong — placeholder untuk slot tab aktif.
    fn empty() -> Doc {
        Doc {
            src: String::new(),
            path: None,
            md_host: None,
            saved_src: String::new(),
            model: Model::Flow(Graph::default()),
            pos: Vec::new(),
            scn: blank_scene(0.0, 0.0),
            tables: Vec::new(),
            cards: Vec::new(),
            boxes: Vec::new(),
            rels: Vec::new(),
            pie: None,
            seq: None,
            pie_labels: Vec::new(),
            pie_empty: false,
            seq_labels: Vec::new(),
            mdoc: None,
            error: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

struct App {
    // Tab dokumen: docs[active] = cangkang; state aslinya ada di
    // field src/path/model/... di bawah (dokumen aktif).
    docs: Vec<Doc>,
    active: usize,
    src: String,
    path: Option<PathBuf>, // file yang sedang dibuka (None = belum disimpan)
    md_host: Option<MdHost>, // Some = dokumen ini blok mermaid di file .md
    saved_src: String,     // isi terakhir yang tersimpan, untuk deteksi dirty
    recent: Vec<String>,   // file terakhir dibuka, terbaru di depan
    workspace: Option<PathBuf>, // folder explorer ala VSCode (panel kiri)
    // Cache isi tiap folder yang sudah dibaca — explorer tak lagi
    // menyentuh filesystem tiap frame. Ok(entries) sudah ter-filter
    // & terurut; Err = pesan gagal baca (mis. folder tercabut).
    // Rc: draw_tree meminjam listing tanpa deep-clone per frame.
    dir_cache: HashMap<PathBuf, Rc<Result<Vec<TreeEntry>, String>>>,
    view: View,               // tab aktif: Preview / Code
    pending: Option<Pending>, // aksi menunggu konfirmasi buang-perubahan
    last_title: String,
    // Kunci perubahan judul jendela (None = belum pernah dihitung).
    last_dirty: Option<bool>,
    last_titled_path: Option<PathBuf>,
    last_titled_tab: usize,
    model: Model,             // dokumen valid terakhir
    pos: Vec<(f64, f64)>,     // posisi node/entitas, milik aplikasi (bisa digeser)
    scn: Scene,               // geometri terkini untuk digambar
    tables: Vec<ErTable>,     // data tabel ER (kosong untuk flowchart)
    cards: Vec<(Card, Card)>, // kardinalitas per relasi ER, sejajar scn.edges
    boxes: Vec<ClassBox>,     // data box class (kosong untuk non-class)
    rels: Vec<RelStyle>,      // gaya/kardinalitas relasi class, sejajar scn.edges
    pie: Option<PieScene>,    // geometri pie (Some hanya untuk Model::Pie)
    seq: Option<SeqScene>,    // geometri sequence (Some hanya untuk Model::Sequence)
    // Precompute tampilan diagram statis (hindari format! per frame):
    pie_labels: Vec<Option<String>>, // "NN%" per slice; None = terlalu tipis
    pie_empty: bool,                 // total 0 → outline saja
    seq_labels: Vec<String>,         // label pesan final ("N. teks" / teks)
    mdoc: Option<MdDoc>,             // Some = tab dokumen Markdown ter-render
    error: Option<String>,
    status: String,
    zoom: f32,         // faktor zoom kanvas (1.0 = 100%)
    pan: Vec2,         // geseran kanvas, piksel layar
    canvas_size: Vec2, // ukuran kanvas frame terakhir (jangkar zoom via tombol)
}

impl App {
    fn new(
        src: String,
        path: Option<PathBuf>,
        recent: Vec<String>,
        workspace: Option<PathBuf>,
    ) -> Self {
        let saved_src = src.clone();
        let mut app = App {
            docs: vec![Doc::empty()],
            active: 0,
            src,
            path,
            md_host: None,
            saved_src,
            recent,
            workspace,
            dir_cache: HashMap::new(),
            view: View::Split,
            pending: None,
            last_title: String::new(),
            last_dirty: None,
            last_titled_path: None,
            last_titled_tab: usize::MAX,
            model: Model::Flow(Graph::default()),
            pos: Vec::new(),
            scn: Scene {
                nodes: Vec::new(),
                edges: Vec::new(),
                clusters: Vec::new(),
                width: 0.0,
                height: 0.0,
            },
            tables: Vec::new(),
            cards: Vec::new(),
            boxes: Vec::new(),
            rels: Vec::new(),
            pie: None,
            seq: None,
            pie_labels: Vec::new(),
            pie_empty: false,
            seq_labels: Vec::new(),
            mdoc: None,
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
    ///
    /// Auto-layout hanya dihitung bila ada node BARU (kunci tak
    /// ditemukan) — saat mengetik biasa semua kunci ketemu, jadi
    /// keystroke tidak membayar layout penuh yang langsung dibuang.
    fn reparse(&mut self) {
        // Dokumen Markdown dirender sebagai DOKUMEN (heading, teks,
        // diagram inline) — bukan diparse sebagai diagram.
        if self.path_is_markdown() {
            self.mdoc = Some(build_mdoc(&self.src));
            self.model = Model::Flow(Graph::default());
            self.pos = Vec::new();
            self.clear_aux();
            self.scn = blank_scene(0.0, 0.0);
            self.error = None;
            return;
        }
        self.mdoc = None;
        match flowmaid::parser::parse_document(&self.src) {
            Ok(doc) => {
                // Model lama ditahan hidup di lokal supaya peta posisi
                // bisa meminjam kuncinya (tanpa alokasi String).
                let prev = std::mem::replace(&mut self.model, Model::Flow(Graph::default()));
                let prev_pos = std::mem::take(&mut self.pos);
                let old: HashMap<&str, (f64, f64)> = prev
                    .keys()
                    .into_iter()
                    .zip(prev_pos.iter().copied())
                    .collect();
                match doc {
                    // State diagram menumpang Graph flowchart —
                    // drag, posisi by-key, dan gambar sama persis.
                    Document::Flowchart(g) | Document::State(g) => {
                        let mut auto = None;
                        self.pos = g
                            .nodes
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                old.get(n.id.as_str()).copied().unwrap_or_else(|| {
                                    let a = auto.get_or_insert_with(|| scene(&g));
                                    (a.nodes[i].x, a.nodes[i].y)
                                })
                            })
                            .collect();
                        self.model = Model::Flow(g);
                    }
                    Document::Er(d) => {
                        let mut auto = None;
                        self.pos = d
                            .entities
                            .iter()
                            .enumerate()
                            .map(|(i, e)| {
                                old.get(e.name.as_str()).copied().unwrap_or_else(|| {
                                    let a = auto.get_or_insert_with(|| er::scene(&d));
                                    (a.scene.nodes[i].x, a.scene.nodes[i].y)
                                })
                            })
                            .collect();
                        self.model = Model::Er(d);
                    }
                    Document::Class(d) => {
                        let mut auto = None;
                        self.pos = d
                            .classes
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                old.get(c.name.as_str()).copied().unwrap_or_else(|| {
                                    let a = auto.get_or_insert_with(|| class::scene(&d));
                                    (a.scene.nodes[i].x, a.scene.nodes[i].y)
                                })
                            })
                            .collect();
                        self.model = Model::Class(d);
                    }
                    // Pie & sequence are static — no per-node positions.
                    Document::Pie(d) => {
                        self.model = Model::Pie(d);
                    }
                    Document::Sequence(d) => {
                        self.model = Model::Sequence(d);
                    }
                }
                self.reroute();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn reroute(&mut self) {
        self.clear_aux();
        match &self.model {
            Model::Flow(g) => self.scn = route(g, &self.pos),
            Model::Er(d) => {
                let es = er::route(d, &self.pos);
                self.scn = es.scene;
                self.tables = es.tables;
                self.cards = es.cards;
            }
            Model::Class(d) => {
                let cs = class::route(d, &self.pos);
                self.scn = cs.scene;
                self.boxes = cs.boxes;
                self.rels = cs.rels;
            }
            // Static: recompute the scene; nothing to route.
            Model::Pie(d) => self.set_static_pie(pie::scene(d)),
            Model::Sequence(d) => self.set_static_seq(seq::scene(d)),
        }
    }

    /// Kembali ke tata letak otomatis engine.
    fn autolayout(&mut self) {
        self.clear_aux();
        match &self.model {
            Model::Flow(g) => self.scn = scene(g),
            Model::Er(d) => {
                let es = er::scene(d);
                self.scn = es.scene;
                self.tables = es.tables;
                self.cards = es.cards;
            }
            Model::Class(d) => {
                let cs = class::scene(d);
                self.scn = cs.scene;
                self.boxes = cs.boxes;
                self.rels = cs.rels;
            }
            Model::Pie(d) => self.set_static_pie(pie::scene(d)),
            Model::Sequence(d) => self.set_static_seq(seq::scene(d)),
        }
        self.pos = self.scn.nodes.iter().map(|n| (n.x, n.y)).collect();
        self.reset_view();
    }

    /// Kosongkan data gambar khusus-diagram (ER, class, pie, seq);
    /// tiap arm route/autolayout mengisi ulang miliknya.
    fn clear_aux(&mut self) {
        self.tables.clear();
        self.cards.clear();
        self.boxes.clear();
        self.rels.clear();
        self.pie = None;
        self.seq = None;
        self.pie_labels.clear();
        self.seq_labels.clear();
    }

    /// Simpan geometri pie statis; `scn` dikosongkan (tak ada node
    /// yang bisa digeser) tapi memegang ukuran kanvas. Label persen
    /// dan flag kosong di-precompute agar draw_pie bebas alokasi.
    fn set_static_pie(&mut self, ps: PieScene) {
        self.pie_labels = ps
            .slices
            .iter()
            .map(|sl| {
                (sl.frac >= pie::MIN_LABEL_FRAC).then(|| format!("{:.0}%", sl.frac * 100.0))
            })
            .collect();
        self.pie_empty = ps.slices.iter().map(|s| s.frac).sum::<f64>() <= f64::EPSILON;
        self.scn = blank_scene(ps.width, ps.height);
        self.pie = Some(ps);
    }

    /// Simpan geometri sequence statis; label pesan final (termasuk
    /// prefiks autonumber) di-precompute sekali, bukan format! per frame.
    fn set_static_seq(&mut self, sc: SeqScene) {
        self.seq_labels = sc
            .messages
            .iter()
            .map(|m| match m.number {
                Some(k) => format!("{k}. {}", m.text),
                None => m.text.clone(),
            })
            .collect();
        self.scn = blank_scene(sc.width, sc.height);
        self.seq = Some(sc);
    }

    /// SVG dari susunan saat ini (termasuk hasil geseran).
    fn export_svg(&self) -> String {
        match &self.model {
            Model::Flow(_) => to_svg(&self.scn),
            Model::Er(d) => er::to_svg(&er::route(d, &self.pos)),
            Model::Class(d) => class::to_svg(&class::route(d, &self.pos)),
            Model::Pie(d) => pie::to_svg(&pie::scene(d)),
            Model::Sequence(d) => seq::to_svg(&seq::scene(d)),
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

    // ── Manajemen tab ─────────────────────────────────────────────

    /// Parkir dokumen aktif ke slot cangkangnya di `docs`.
    fn park(&mut self) {
        let d = &mut self.docs[self.active];
        d.src = std::mem::take(&mut self.src);
        d.path = self.path.take();
        d.md_host = self.md_host.take();
        d.saved_src = std::mem::take(&mut self.saved_src);
        d.model = std::mem::replace(&mut self.model, Model::Flow(Graph::default()));
        d.pos = std::mem::take(&mut self.pos);
        d.scn = std::mem::replace(&mut self.scn, blank_scene(0.0, 0.0));
        d.tables = std::mem::take(&mut self.tables);
        d.cards = std::mem::take(&mut self.cards);
        d.boxes = std::mem::take(&mut self.boxes);
        d.rels = std::mem::take(&mut self.rels);
        d.pie = self.pie.take();
        d.seq = self.seq.take();
        d.pie_labels = std::mem::take(&mut self.pie_labels);
        d.pie_empty = self.pie_empty;
        d.seq_labels = std::mem::take(&mut self.seq_labels);
        d.mdoc = self.mdoc.take();
        d.error = self.error.take();
        d.zoom = self.zoom;
        d.pan = self.pan;
    }

    /// Muat dokumen ke-`i` dari parkiran menjadi dokumen aktif
    /// (isi lama field aktif DIBUANG — parkir dulu bila perlu).
    fn load(&mut self, i: usize) {
        self.active = i;
        let d = &mut self.docs[i];
        self.src = std::mem::take(&mut d.src);
        self.path = d.path.take();
        self.md_host = d.md_host.take();
        self.saved_src = std::mem::take(&mut d.saved_src);
        self.model = std::mem::replace(&mut d.model, Model::Flow(Graph::default()));
        self.pos = std::mem::take(&mut d.pos);
        self.scn = std::mem::replace(&mut d.scn, blank_scene(0.0, 0.0));
        self.tables = std::mem::take(&mut d.tables);
        self.cards = std::mem::take(&mut d.cards);
        self.boxes = std::mem::take(&mut d.boxes);
        self.rels = std::mem::take(&mut d.rels);
        self.pie = d.pie.take();
        self.seq = d.seq.take();
        self.pie_labels = std::mem::take(&mut d.pie_labels);
        self.pie_empty = d.pie_empty;
        self.seq_labels = std::mem::take(&mut d.seq_labels);
        self.mdoc = d.mdoc.take();
        self.error = d.error.take();
        self.zoom = d.zoom;
        self.pan = d.pan;
    }

    fn switch_to(&mut self, i: usize) {
        if i != self.active && i < self.docs.len() {
            self.park();
            self.load(i);
        }
    }

    /// Judul tab ke-`i` (nama file / blok md / "tanpa judul") — tab
    /// aktif dibaca dari field live, bukan dari cangkangnya.
    fn tab_title(&self, i: usize) -> String {
        let (path, md, dirty) = if i == self.active {
            (&self.path, &self.md_host, self.dirty())
        } else {
            let d = &self.docs[i];
            (&d.path, &d.md_host, d.src != d.saved_src)
        };
        let name = match (path, md) {
            (Some(p), _) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "tanpa judul".into()),
            (None, Some(h)) => format!(
                "{} #{}",
                h.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                h.index + 1
            ),
            (None, None) => "tanpa judul".into(),
        };
        if dirty {
            format!("{name} •")
        } else {
            name
        }
    }

    /// Dokumen aktif masih persis contoh bawaan yang belum disentuh?
    /// (Dipakai supaya membuka file tak meninggalkan tab sampah.)
    fn active_is_pristine_untitled(&self) -> bool {
        self.path.is_none() && !self.dirty() && self.src == CONTOH
    }

    /// Satu tab di bar: judul + tombol tutup dalam SATU pil membulat,
    /// tab aktif diberi warna seleksi. Klik-tengah juga menutup.
    fn draw_tab(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        switch: &mut Option<usize>,
        close: &mut Option<usize>,
    ) {
        let active = i == self.active;
        let (fill, text_color) = if active {
            (ui.visuals().selection.bg_fill, ui.visuals().selection.stroke.color)
        } else {
            (ui.visuals().faint_bg_color, ui.visuals().weak_text_color())
        };
        egui::Frame::none()
            .fill(fill)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(9.0, 4.0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let title = ui.add(
                    egui::Label::new(egui::RichText::new(self.tab_title(i)).color(text_color))
                        .sense(egui::Sense::click())
                        .selectable(false),
                );
                if title.clicked() && !active {
                    *switch = Some(i);
                }
                if title.middle_clicked() {
                    *close = Some(i);
                }
                let x = ui.add(
                    egui::Button::new(egui::RichText::new("x").size(11.0).color(text_color))
                        .frame(false),
                );
                if x.on_hover_text("tutup tab (Cmd+W)").clicked() {
                    *close = Some(i);
                }
            });
    }

    /// Dokumen aktif adalah file Markdown? (mode dokumen ter-render)
    fn path_is_markdown(&self) -> bool {
        self.path
            .as_ref()
            .and_then(|p| p.extension())
            .map(|e| {
                let e = e.to_string_lossy().to_lowercase();
                e == "md" || e == "markdown"
            })
            .unwrap_or(false)
    }

    fn open_path(&mut self, p: PathBuf) {
        // Canonicalize agar cocok dengan path pohon explorer
        // (highlight file aktif) dan untuk dedupe antar-tab.
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        // Sudah terbuka? Aktifkan tabnya saja.
        if self.path.as_ref() == Some(&p) {
            return;
        }
        if let Some(i) = (0..self.docs.len())
            .filter(|&i| i != self.active)
            .find(|&i| self.docs[i].path.as_ref() == Some(&p))
        {
            self.switch_to(i);
            self.status = format!("pindah ke: {}", p.display());
            return;
        }
        match std::fs::read_to_string(&p) {
            Ok(t) => {
                // Tab contoh bawaan yang belum disentuh ditimpa di
                // tempat; selain itu file baru dibuka di TAB BARU.
                if !self.active_is_pristine_untitled() {
                    self.park();
                    self.docs.push(Doc::empty());
                    self.active = self.docs.len() - 1;
                }
                self.src = t;
                self.saved_src = self.src.clone();
                // Path di-set SEBELUM reparse — mode dokumen Markdown
                // ditentukan dari ekstensinya.
                self.path = Some(p.clone());
                self.reparse();
                self.reset_view();
                self.status = format!("dibuka: {}", p.display());
                self.push_recent(&p);
            }
            Err(e) => self.status = format!("gagal membuka: {}", e),
        }
    }

    /// Buka SATU blok ```mermaid dari file Markdown sebagai tab
    /// diagram tersendiri (tombol "edit" di tampilan dokumen).
    /// Menyimpan tab tersebut menulis balik ke dalam fence-nya.
    fn open_md_block(&mut self, host: &Path, index: usize, src: String) {
        let host_eq = |h: &Option<MdHost>| {
            h.as_ref().is_some_and(|h| h.path == host && h.index == index)
        };
        // Dedupe: blok ini sudah punya tab? Aktifkan saja.
        if host_eq(&self.md_host) {
            return;
        }
        if let Some(t) = (0..self.docs.len())
            .filter(|&t| t != self.active)
            .find(|&t| host_eq(&self.docs[t].md_host))
        {
            self.switch_to(t);
            return;
        }
        if !self.active_is_pristine_untitled() {
            self.park();
            self.docs.push(Doc::empty());
            self.active = self.docs.len() - 1;
        }
        self.src = src;
        self.saved_src = self.src.clone();
        self.path = None;
        self.md_host = Some(MdHost {
            path: host.to_path_buf(),
            index,
        });
        self.reparse();
        self.reset_view();
        self.status = format!("blok #{} dari {}", index + 1, host.display());
    }

    /// Tab baru berisi contoh bawaan.
    fn new_file(&mut self) {
        self.park();
        self.docs.push(Doc::empty());
        self.active = self.docs.len() - 1;
        self.src = CONTOH.to_string();
        self.saved_src = self.src.clone();
        self.path = None;
        self.reparse();
        self.reset_view();
        self.status = "dokumen baru".into();
    }

    /// Minta tutup tab ke-`i`; tab dirty ditahan di dialog konfirmasi.
    /// Tab non-aktif diaktifkan dulu supaya "Simpan dulu" di dialog
    /// bekerja pada dokumen yang benar.
    fn request_close(&mut self, i: usize) {
        if self.pending.is_some() || i >= self.docs.len() {
            return;
        }
        self.switch_to(i);
        if self.dirty() {
            self.pending = Some(Pending::CloseTab(i));
        } else {
            self.close_tab(i);
        }
    }

    /// Tutup tab ke-`i` (harus tab aktif — dijamin `request_close`).
    /// Tab terakhir tidak ditutup, melainkan diganti dokumen baru.
    fn close_tab(&mut self, i: usize) {
        if self.docs.len() <= 1 {
            self.src = CONTOH.to_string();
            self.saved_src = self.src.clone();
            self.path = None;
            self.reparse();
            self.reset_view();
            self.status = "dokumen baru".into();
            return;
        }
        // State asli tab aktif ada di field live — cukup buang
        // cangkangnya lalu muat tetangga.
        self.docs.remove(i);
        self.load(i.min(self.docs.len() - 1));
        self.status = "tab ditutup".into();
    }

    /// Simpan ke file saat ini; blok Markdown menulis balik ke dalam
    /// fence-nya; belum punya file → Simpan Sebagai. Mengembalikan
    /// `true` bila dokumen benar-benar tersimpan.
    fn save_doc(&mut self) -> bool {
        if self.md_host.is_some() {
            return self.save_md_block();
        }
        match self.path.clone() {
            Some(p) => self.write_to(&p),
            None => self.save_as(),
        }
    }

    /// Tulis isi editor balik KE DALAM fence ```mermaid asalnya.
    /// File induk dibaca ulang dan bloknya dicari lagi saat menyimpan,
    /// jadi suntingan lain pada file (di luar blok ini) tidak hilang.
    fn save_md_block(&mut self) -> bool {
        let Some(host) = self.md_host.clone() else { return false };
        let md = match std::fs::read_to_string(&host.path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("gagal menyimpan: {}", e);
                return false;
            }
        };
        let Some(next) = splice_md_block(&md, host.index, &self.src) else {
            self.status = format!(
                "gagal menyimpan: blok mermaid #{} tidak ditemukan lagi di {} \
                 (file berubah, atau fence-nya ter-indentasi)",
                host.index + 1,
                host.path.display()
            );
            return false;
        };
        match std::fs::write(&host.path, next) {
            Ok(_) => {
                self.saved_src = self.src.clone();
                self.status = format!(
                    "tersimpan ke blok #{} di {}",
                    host.index + 1,
                    host.path.display()
                );
                true
            }
            Err(e) => {
                self.status = format!("gagal menyimpan: {}", e);
                false
            }
        }
    }

    /// `true` bila tersimpan; `false` bila dibatalkan atau gagal tulis.
    fn save_as(&mut self) -> bool {
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
        match dlg.save_file() {
            Some(p) if self.write_to(&p) => {
                self.push_recent(&p);
                self.path = Some(p);
                // Simpan-Sebagai melepaskan dokumen dari file .md
                // induknya — ia kini file .mmd mandiri.
                self.md_host = None;
                true
            }
            _ => false,
        }
    }

    fn write_to(&mut self, p: &Path) -> bool {
        match std::fs::write(p, &self.src) {
            Ok(_) => {
                self.saved_src = self.src.clone();
                self.status = format!("tersimpan: {}", p.display());
                // Explorer membaca dari dir_cache — tanpa invalidasi,
                // file baru hasil Simpan-Sebagai tak pernah muncul di
                // pohon sampai "Segarkan" manual (temuan bug hunt).
                if let Some(parent) = p.parent() {
                    self.dir_cache.remove(parent);
                    // Path pohon ter-canonicalize; path simpan belum tentu.
                    if let Ok(canon) = std::fs::canonicalize(parent) {
                        self.dir_cache.remove(&canon);
                    }
                }
                true
            }
            Err(e) => {
                self.status = format!("gagal menyimpan: {}", e);
                false
            }
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

    /// Pilih folder untuk panel explorer (ala VSCode).
    fn open_folder_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new();
        if let Some(ws) = &self.workspace {
            dlg = dlg.set_directory(ws);
        }
        if let Some(p) = dlg.pick_folder() {
            // Canonicalize supaya path anak di pohon sejajar dengan
            // `self.path` (yang juga di-canonicalize) → highlight jalan.
            let p = std::fs::canonicalize(&p).unwrap_or(p);
            self.status = format!("folder: {}", p.display());
            self.dir_cache.clear();
            self.workspace = Some(p);
        }
    }

    /// Isi satu folder (folder-dulu, alfabetis, ter-filter),
    /// di-cache supaya explorer tak menyentuh filesystem tiap frame.
    /// Symlink direktori dilewati agar tak ada siklus tak berujung.
    fn listing(&mut self, dir: &Path) -> Rc<Result<Vec<TreeEntry>, String>> {
        if let Some(cached) = self.dir_cache.get(dir) {
            return Rc::clone(cached);
        }
        // Penjaga pertumbuhan: cache tak pernah dievict selama sesi
        // (folder tertutup tetap tersimpan). Reset kasar saat besar;
        // frame berikutnya mengisi ulang hanya yang terlihat.
        if self.dir_cache.len() > 512 {
            self.dir_cache.clear();
        }
        let is_symlink = |p: &Path| {
            std::fs::symlink_metadata(p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        };
        let result = std::fs::read_dir(dir).map_err(|e| e.to_string()).map(|rd| {
            let mut v: Vec<TreeEntry> = rd
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    if name.starts_with('.') || name == "target" {
                        return None;
                    }
                    // Folder asli (bukan symlink) atau file diagram —
                    // is_dir dihitung SEKALI di sini, bukan per frame.
                    let is_dir = path.is_dir() && !is_symlink(&path);
                    let lower = name.to_lowercase();
                    if is_dir
                        || lower.ends_with(".mmd")
                        || lower.ends_with(".txt")
                        || lower.ends_with(".md")
                        || lower.ends_with(".markdown")
                    {
                        Some(TreeEntry { path, name, is_dir })
                    } else {
                        None
                    }
                })
                .collect();
            v.sort_by_cached_key(|t| (!t.is_dir, t.name.to_lowercase()));
            v
        });
        let rc = Rc::new(result);
        self.dir_cache.insert(dir.to_path_buf(), Rc::clone(&rc));
        rc
    }

    /// Pohon file rekursif untuk explorer: folder bisa dilipat,
    /// file `.mmd`/`.txt` bisa diklik untuk dibuka (lewat penjaga
    /// perubahan-belum-disimpan), file aktif di-highlight.
    fn draw_tree(&mut self, ui: &mut egui::Ui, dir: &Path) {
        // Rc lokal menahan data hidup — rekursi &mut self tetap aman
        // walau cache dievict di tengah jalan.
        let listing = self.listing(dir);
        let entries = match listing.as_ref() {
            Ok(v) => v,
            Err(e) => {
                ui.colored_label(
                    Color32::from_rgb(200, 60, 60),
                    format!("⚠ folder tidak dapat dibaca: {e}"),
                );
                return;
            }
        };
        for t in entries {
            if t.is_dir {
                egui::CollapsingHeader::new(&t.name)
                    .id_salt(&t.path)
                    .default_open(false)
                    .show(ui, |ui| self.draw_tree(ui, &t.path));
            } else {
                let selected = self.path.as_deref() == Some(t.path.as_path());
                if ui.selectable_label(selected, &t.name).clicked() && !selected {
                    self.open_path(t.path.clone());
                }
            }
        }
    }

    fn perform(&mut self, act: Pending) {
        match act {
            Pending::CloseTab(i) => self.close_tab(i),
        }
    }

    /// Dialog buka file — hasilnya jadi tab (tak ada yang terbuang).
    fn open_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new()
            .add_filter("Diagram", &["mmd", "txt", "md", "markdown"])
            .add_filter("Markdown", &["md", "markdown"]);
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dlg = dlg.set_directory(dir);
        }
        if let Some(p) = dlg.pick_file() {
            self.open_path(p);
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("recent", self.recent.join("\n"));
        storage.set_string(
            "workspace",
            self.workspace
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        );
        // Tab yang punya file dibuka lagi di sesi berikutnya
        // (dokumen tanpa judul tidak — isinya tak dipersist).
        let tabs: Vec<String> = (0..self.docs.len())
            .filter_map(|i| {
                let p = if i == self.active { &self.path } else { &self.docs[i].path };
                p.as_ref().map(|p| p.display().to_string())
            })
            .collect();
        storage.set_string("tabs", tabs.join("\n"));
    }

    // Catatan audit: persist_egui_memory sengaja DIBIARKAN default
    // (true) — mematikannya ikut menghilangkan lebar panel, posisi
    // scroll, dan UI zoom antar-restart, dan merusak restorasi
    // geometri window saat user pernah ⌘+/− (temuan verifikasi).

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Shortcut file. Simpan-Sebagai dicek sebelum Simpan supaya
        // ⇧⌘S tidak termakan ⌘S.
        use egui::{Key, KeyboardShortcut, Modifiers};
        const SAVE_AS: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
        const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        const OPEN_FOLDER: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
        const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
        const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE_AS)) {
            self.save_as();
        } else if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.save_doc();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&OPEN_FOLDER)) {
            self.open_folder_dialog();
        } else if ctx.input_mut(|i| i.consume_shortcut(&OPEN)) {
            self.open_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&NEW)) {
            self.new_file();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&CLOSE_TAB)) {
            self.request_close(self.active);
        }

        // Drag & drop FILE .mmd ke jendela — tiap file jadi tab.
        let dropped: Vec<PathBuf> = ctx
            .input(|i| i.raw.dropped_files.clone())
            .into_iter()
            .filter_map(|f| f.path)
            .collect();
        for p in dropped {
            self.open_path(p);
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
                            // Lanjut HANYA bila benar-benar tersimpan.
                            // Gagal tulis atau Simpan-Sebagai dibatalkan
                            // → dialog tetap terbuka, aksi tak hilang.
                            if self.save_doc() {
                                decided = Some(self.pending.take());
                            }
                        }
                        if ui.button("Buang perubahan").clicked() {
                            decided = Some(self.pending.take());
                        }
                        if ui.button("Batal").clicked() {
                            decided = Some(None);
                        }
                    });
                    // Tampilkan error simpan DI DALAM dialog, bukan di
                    // baris status yang tertutup dialog ini.
                    if self.status.starts_with("gagal") {
                        ui.add_space(6.0);
                        ui.colored_label(Color32::from_rgb(200, 60, 60), &self.status);
                    }
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
                    if ui.button("Tab Baru  (Cmd+N)").clicked() {
                        self.new_file();
                        ui.close_menu();
                    }
                    if ui.button("Buka…  (Cmd+O)").clicked() {
                        self.open_dialog();
                        ui.close_menu();
                    }
                    if ui.button("Buka Folder…  (Shift+Cmd+O)").clicked() {
                        self.open_folder_dialog();
                        ui.close_menu();
                    }
                    ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
                        ui.menu_button("Baru dibuka", |ui| {
                            for r in self.recent.clone() {
                                if ui.button(&r).clicked() {
                                    self.open_path(PathBuf::from(&r));
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                    if ui.button("Tutup Tab  (Cmd+W)").clicked() {
                        self.request_close(self.active);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Simpan  (Cmd+S)").clicked() {
                        self.save_doc();
                        ui.close_menu();
                    }
                    if ui.button("Simpan Sebagai…  (Shift+Cmd+S)").clicked() {
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

        // Bar tab dokumen — klik pindah, klik-tengah / "x" menutup,
        // "+" tab baru. Satu baris ber-scroll, tidak melipat.
        egui::TopBottomPanel::top("tabbar")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal()
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut switch: Option<usize> = None;
                            let mut close: Option<usize> = None;
                            for i in 0..self.docs.len() {
                                self.draw_tab(ui, i, &mut switch, &mut close);
                            }
                            ui.add_space(2.0);
                            if ui
                                .add(egui::Button::new("+").frame(false))
                                .on_hover_text("tab baru (Cmd+N)")
                                .clicked()
                            {
                                self.new_file();
                            }
                            if let Some(i) = switch {
                                self.switch_to(i);
                            }
                            if let Some(i) = close {
                                self.request_close(i);
                            }
                        });
                    });
            });

        // Explorer folder ala VSCode — panel kiri, selalu tampil.
        egui::SidePanel::left("explorer")
            .resizable(true)
            .default_width(220.0)
            .width_range(160.0..=440.0)
            .show(ctx, |ui| match self.workspace.clone() {
                Some(ws) => {
                    ui.horizontal(|ui| {
                        // Menu aksi dipasang lebih dulu di kanan, lalu
                        // nama folder mengisi sisa ruang & terpotong
                        // rapi bila panjang.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.menu_button("...", |ui| {
                                if ui.button("Segarkan").clicked() {
                                    self.dir_cache.clear();
                                    ui.close_menu();
                                }
                                if ui.button("Tutup folder").clicked() {
                                    self.workspace = None;
                                    self.dir_cache.clear();
                                    ui.close_menu();
                                }
                            });
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        ws.file_name()
                                            .map(|n| n.to_string_lossy().to_uppercase())
                                            .unwrap_or_else(|| "FOLDER".into()),
                                    )
                                    .strong(),
                                )
                                .truncate(),
                            );
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.draw_tree(ui, &ws);
                    });
                }
                None => {
                    ui.add_space(8.0);
                    ui.label("Belum ada folder terbuka.");
                    ui.add_space(6.0);
                    if ui.button("Buka Folder…").clicked() {
                        self.open_folder_dialog();
                    }
                    ui.add_space(4.0);
                    ui.small("Shift+Cmd+O untuk menjelajahi file .mmd.");
                }
            });

        // Toolbar: mode tampilan di kiri, aksi, zoom rata-kanan.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view, View::Preview, "Preview");
                ui.selectable_value(&mut self.view, View::Split, "Split");
                ui.selectable_value(&mut self.view, View::Code, "Code");
                ui.separator();
                // Aksi kanvas tak relevan untuk tab dokumen Markdown.
                if self.mdoc.is_none() {
                    if ui
                        .button("Tata ulang")
                        .on_hover_text("tata letak otomatis")
                        .clicked()
                    {
                        self.autolayout();
                    }
                    if ui.button("Ekspor SVG").clicked() {
                        self.export_svg_file();
                    }
                }
                if self.view != View::Code && self.mdoc.is_none() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if ui.small_button("+").clicked() {
                            self.zoom_around(1.25, self.canvas_size / 2.0);
                        }
                        if ui
                            .small_button(format!("{:.0}%", self.zoom * 100.0))
                            .on_hover_text("reset tampilan")
                            .clicked()
                        {
                            self.reset_view();
                        }
                        if ui.small_button("-").clicked() {
                            self.zoom_around(1.0 / 1.25, self.canvas_size / 2.0);
                        }
                    });
                }
            });
        });

        // Status bar bawah yang tipis: status/error di kiri, tipe
        // diagram dokumen aktif di kanan.
        egui::TopBottomPanel::bottom("statusbar")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(8.0, 3.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match &self.error {
                        Some(e) => {
                            ui.colored_label(
                                Color32::from_rgb(224, 90, 90),
                                format!("parse: {e}"),
                            );
                        }
                        None => {
                            ui.weak(&self.status);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let kind = if let Some(m) = &self.mdoc {
                            format!("markdown · {} diagram", m.blocks.len())
                        } else {
                            match &self.model {
                                Model::Flow(g) => {
                                    format!("flowchart · {} node", g.nodes.len())
                                }
                                Model::Er(d) => format!("ER · {} entitas", d.entities.len()),
                                Model::Class(d) => format!("class · {} kelas", d.classes.len()),
                                Model::Sequence(d) => {
                                    format!("sequence · {} partisipan", d.participants.len())
                                }
                                Model::Pie(d) => format!("pie · {} slice", d.slices.len()),
                            }
                        };
                        ui.weak(kind);
                    });
                });
            });

        // Area utama sesuai mode aktif.
        match self.view {
            View::Code => {
                egui::CentralPanel::default().show(ctx, |ui| self.draw_editor(ui));
            }
            View::Split => {
                egui::SidePanel::right("code")
                    .resizable(true)
                    .default_width(400.0)
                    .width_range(240.0..=900.0)
                    .show(ctx, |ui| self.draw_editor(ui));
                egui::CentralPanel::default().show(ctx, |ui| self.draw_canvas(ui));
            }
            View::Preview => {
                egui::CentralPanel::default().show(ctx, |ui| self.draw_canvas(ui));
            }
        }

        self.sync_title(ctx);
    }
}

impl App {
    /// Editor teks Mermaid (tab Code / sisi Split).
    fn draw_editor(&mut self, ui: &mut egui::Ui) {
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
    }

    /// Dokumen Markdown ter-render: heading/teks/list/kutipan/kode
    /// plus tiap blok ```mermaid dilukis inline sebagai diagram.
    fn draw_document(&mut self, ui: &mut egui::Ui) {
        // Ambil-sementara supaya iterasi item tak bentrok dengan
        // aksi tombol yang butuh &mut self.
        let Some(mdoc) = self.mdoc.take() else { return };
        let mut open_req: Option<usize> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(20.0, 16.0))
                    .show(ui, |ui| {
                        ui.set_max_width(860.0);
                        for item in &mdoc.items {
                            draw_md_item(ui, item, &mdoc, &mut open_req);
                        }
                        ui.add_space(24.0);
                    });
            });
        self.mdoc = Some(mdoc);
        if let Some(i) = open_req {
            if let (Some(host), Some(src)) = (
                self.path.clone(),
                self.mdoc.as_ref().and_then(|m| m.block_srcs.get(i)).cloned(),
            ) {
                self.open_md_block(&host, i, src);
            }
        }
    }

    /// Kanvas diagram interaktif (tab Preview / sisi Split).
    fn draw_canvas(&mut self, ui: &mut egui::Ui) {
        // Tab dokumen Markdown memakai tampilan dokumen, bukan kanvas.
        if self.mdoc.is_some() {
            self.draw_document(ui);
            return;
        }
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

        // 1) Interaksi drag NODE dulu (rect dalam koordinat layar) —
        //    rect dihitung inline, tanpa Vec perantara per frame.
        let mut moved = false;
        let mut hovered_node: Option<usize> = None;
        for i in 0..self.scn.nodes.len() {
            let n = &self.scn.nodes[i];
            let rect =
                Rect::from_center_size(ts(n.x, n.y), Vec2::new(n.w as f32, n.h as f32) * zoom);
            let resp = ui.interact(rect, egui::Id::new(("flowrs-node", i)), Sense::drag());
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

        // 2) Lukis diagram lewat fungsi bersama (juga dipakai
        //    pratinjau inline dokumen Markdown).
        let painter = ui.painter();
        let refs = DiagramRefs {
            scn: &self.scn,
            tables: &self.tables,
            cards: &self.cards,
            boxes: &self.boxes,
            rels: &self.rels,
            pie: self.pie.as_ref(),
            pie_labels: &self.pie_labels,
            pie_empty: self.pie_empty,
            seq: self.seq.as_ref(),
            seq_labels: &self.seq_labels,
        };
        paint_diagram(painter, &refs, hovered_node, &ts, zoom, true);
    }

    /// Judul jendela dihitung di AKHIR frame — setelah semua mutasi
    /// state — supaya indikator dirty tidak telat satu frame sesudah
    /// ⌘S (ditemukan bughunter). String judul hanya dibangun ulang
    /// saat (dirty, path) berubah — bukan format! tiap frame.
    fn sync_title(&mut self, ctx: &egui::Context) {
        let dirty = self.dirty();
        if Some(dirty) == self.last_dirty
            && self.path == self.last_titled_path
            && self.active == self.last_titled_tab
        {
            return;
        }
        self.last_dirty = Some(dirty);
        self.last_titled_path.clone_from(&self.path);
        self.last_titled_tab = self.active;
        // tab_title sudah menangani ketiga bentuk nama (file, blok
        // md, tanpa judul); buang dot dirty-nya karena judul window
        // memakai prefiks.
        let name = self.tab_title(self.active);
        let name = name.strip_suffix(" •").unwrap_or(&name);
        let title = format!(
            "{}{} — flowmaid desktop",
            if dirty { "• " } else { "" },
            name
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

// ── Markdown: dokumen ter-render dengan diagram inline ────────────

/// Referensi data gambar SATU diagram — dipinjam dari field App
/// (kanvas utama) atau dari blok Markdown (pratinjau inline), supaya
/// keduanya memakai satu fungsi lukis yang sama: [`paint_diagram`].
struct DiagramRefs<'a> {
    scn: &'a Scene,
    tables: &'a [ErTable],
    cards: &'a [(Card, Card)],
    boxes: &'a [ClassBox],
    rels: &'a [RelStyle],
    pie: Option<&'a PieScene>,
    pie_labels: &'a [Option<String>],
    pie_empty: bool,
    seq: Option<&'a SeqScene>,
    seq_labels: &'a [String],
}

/// Lukis satu diagram lengkap (kertas, cluster, edge, node/tabel/
/// box, pie, sequence) lewat transform `ts` — dipakai kanvas utama
/// dan pratinjau inline dokumen Markdown.
fn paint_diagram(
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
        painter.rect(paper, 8.0 * zoom, Color32::WHITE, Stroke::new(1.0, LABEL_BORDER));
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
        let p = e.bezier.map(|(x, y)| ts(x, y));
        let sw = (if matches!(e.kind, EdgeKind::Thick | EdgeKind::ThickOpen) {
            3.4
        } else {
            1.7
        }) * zoom;
        let stroke = Stroke::new(sw, EDGE);
        if matches!(e.kind, EdgeKind::Dotted | EdgeKind::DottedOpen) {
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
        if let Some((t, (lx, ly), lw)) = &e.label {
            let c = ts(*lx, *ly);
            let r = Rect::from_center_size(c, Vec2::new(*lw as f32, 20.0) * zoom);
            painter.rect(r, 4.0 * zoom, Color32::WHITE, Stroke::new(1.0 * zoom, LABEL_BORDER));
            painter.text(c, Align2::CENTER_CENTER, t, zfont(13.0, zoom), TEXT);
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

/// Satu blok ```mermaid di dokumen Markdown yang sudah di-layout,
/// siap dilukis inline — atau pesan parse error-nya.
enum BlockView {
    Ok(Box<DiagramData>),
    Err(String),
}

/// Geometri + data gambar milik satu blok (bentuk owned dari
/// [`DiagramRefs`]).
struct DiagramData {
    scn: Scene,
    tables: Vec<ErTable>,
    cards: Vec<(Card, Card)>,
    boxes: Vec<ClassBox>,
    rels: Vec<RelStyle>,
    pie: Option<PieScene>,
    pie_labels: Vec<Option<String>>,
    pie_empty: bool,
    seq: Option<SeqScene>,
    seq_labels: Vec<String>,
    width: f64,
    height: f64,
}

impl DiagramData {
    fn refs(&self) -> DiagramRefs<'_> {
        DiagramRefs {
            scn: &self.scn,
            tables: &self.tables,
            cards: &self.cards,
            boxes: &self.boxes,
            rels: &self.rels,
            pie: self.pie.as_ref(),
            pie_labels: &self.pie_labels,
            pie_empty: self.pie_empty,
            seq: self.seq.as_ref(),
            seq_labels: &self.seq_labels,
        }
    }
}

/// Layout otomatis satu blok mermaid (semua tipe diagram engine).
fn build_block(src: &str) -> BlockView {
    let mut d = DiagramData {
        scn: blank_scene(0.0, 0.0),
        tables: Vec::new(),
        cards: Vec::new(),
        boxes: Vec::new(),
        rels: Vec::new(),
        pie: None,
        pie_labels: Vec::new(),
        pie_empty: false,
        seq: None,
        seq_labels: Vec::new(),
        width: 0.0,
        height: 0.0,
    };
    match flowmaid::parser::parse_document(src) {
        Err(e) => return BlockView::Err(e.to_string()),
        Ok(Document::Flowchart(g)) | Ok(Document::State(g)) => d.scn = scene(&g),
        Ok(Document::Er(er_d)) => {
            let es = er::scene(&er_d);
            d.scn = es.scene;
            d.tables = es.tables;
            d.cards = es.cards;
        }
        Ok(Document::Class(c)) => {
            let cs = class::scene(&c);
            d.scn = cs.scene;
            d.boxes = cs.boxes;
            d.rels = cs.rels;
        }
        Ok(Document::Pie(p)) => {
            let ps = pie::scene(&p);
            d.pie_labels = ps
                .slices
                .iter()
                .map(|sl| {
                    (sl.frac >= pie::MIN_LABEL_FRAC).then(|| format!("{:.0}%", sl.frac * 100.0))
                })
                .collect();
            d.pie_empty = ps.slices.iter().map(|s| s.frac).sum::<f64>() <= f64::EPSILON;
            d.scn = blank_scene(ps.width, ps.height);
            d.pie = Some(ps);
        }
        Ok(Document::Sequence(s)) => {
            let sc = seq::scene(&s);
            d.seq_labels = sc
                .messages
                .iter()
                .map(|m| match m.number {
                    Some(k) => format!("{k}. {}", m.text),
                    None => m.text.clone(),
                })
                .collect();
            d.scn = blank_scene(sc.width, sc.height);
            d.seq = Some(sc);
        }
    }
    d.width = d.scn.width;
    d.height = d.scn.height;
    BlockView::Ok(Box::new(d))
}

/// Satu potongan teks inline dengan gayanya.
struct Run {
    text: String,
    strong: bool,
    em: bool,
    code: bool,
    link: bool,
}

/// Satu elemen blok dokumen Markdown ter-render.
enum MdItem {
    Heading(u8, Vec<Run>),
    Para(Vec<Run>),
    /// (kedalaman indent, nomor urut bila ordered, isi)
    Bullet(u8, Option<u64>, Vec<Run>),
    Quote(Vec<Run>),
    CodeBlock(String),
    /// Indeks ke [`MdDoc::blocks`].
    Diagram(usize),
    Rule,
}

/// Dokumen Markdown ter-render: daftar elemen + diagram ter-layout.
struct MdDoc {
    items: Vec<MdItem>,
    blocks: Vec<BlockView>,
    /// Sumber tiap blok, untuk tombol "edit sebagai tab".
    block_srcs: Vec<String>,
}

/// Bangun [`MdDoc`] dari teks Markdown lewat AST crate `markdown`.
fn build_mdoc(md: &str) -> MdDoc {
    use markdown::mdast::Node;

    fn runs(nodes: &[Node], out: &mut Vec<Run>, strong: bool, em: bool, code: bool, link: bool) {
        for n in nodes {
            match n {
                Node::Text(t) => out.push(Run {
                    text: t.value.clone(),
                    strong,
                    em,
                    code,
                    link,
                }),
                Node::InlineCode(c) => out.push(Run {
                    text: c.value.clone(),
                    strong,
                    em,
                    code: true,
                    link,
                }),
                Node::Strong(s) => runs(&s.children, out, true, em, code, link),
                Node::Emphasis(e) => runs(&e.children, out, strong, true, code, link),
                Node::Link(l) => runs(&l.children, out, strong, em, code, true),
                Node::Delete(d) => runs(&d.children, out, strong, em, code, link),
                Node::Break(_) => out.push(Run {
                    text: "\n".into(),
                    strong,
                    em,
                    code,
                    link,
                }),
                Node::Image(i) => out.push(Run {
                    text: format!("[gambar: {}]", i.alt),
                    strong,
                    em: true,
                    code,
                    link,
                }),
                other => {
                    if let Some(ch) = other.children() {
                        runs(ch, out, strong, em, code, link);
                    }
                }
            }
        }
    }
    fn inline(nodes: &[Node]) -> Vec<Run> {
        let mut v = Vec::new();
        runs(nodes, &mut v, false, false, false, false);
        v
    }
    fn walk(nodes: &[Node], doc: &mut MdDoc, depth: u8) {
        for n in nodes {
            match n {
                Node::Heading(h) => doc.items.push(MdItem::Heading(h.depth, inline(&h.children))),
                Node::Paragraph(p) => {
                    let r = inline(&p.children);
                    if depth == 0 {
                        doc.items.push(MdItem::Para(r));
                    } else {
                        // Paragraf di dalam list item digambar oleh
                        // cabang List di bawah; tak sampai ke sini.
                        doc.items.push(MdItem::Para(r));
                    }
                }
                Node::Code(c) => {
                    let lang = c.lang.as_deref().unwrap_or("");
                    if lang.eq_ignore_ascii_case("mermaid") || lang.eq_ignore_ascii_case("mmd") {
                        doc.items.push(MdItem::Diagram(doc.blocks.len()));
                        doc.blocks.push(build_block(&c.value));
                        doc.block_srcs.push(c.value.clone());
                    } else {
                        doc.items.push(MdItem::CodeBlock(c.value.clone()));
                    }
                }
                Node::List(l) => {
                    let mut num = l.start.map(u64::from);
                    for item in &l.children {
                        if let Node::ListItem(li) = item {
                            // Baris pertama item = paragraf pertamanya.
                            let mut lead: Vec<Run> = Vec::new();
                            for ch in &li.children {
                                match ch {
                                    Node::Paragraph(p) if lead.is_empty() => {
                                        lead = inline(&p.children)
                                    }
                                    Node::List(_) => {}
                                    _ => {}
                                }
                            }
                            doc.items.push(MdItem::Bullet(depth, num, lead));
                            if let Some(k) = num.as_mut() {
                                *k += 1;
                            }
                            // List bersarang di dalam item.
                            for ch in &li.children {
                                if matches!(ch, Node::List(_)) {
                                    walk(std::slice::from_ref(ch), doc, depth + 1);
                                }
                            }
                        }
                    }
                }
                Node::Blockquote(b) => {
                    for ch in &b.children {
                        if let Node::Paragraph(p) = ch {
                            doc.items.push(MdItem::Quote(inline(&p.children)));
                        }
                    }
                }
                Node::ThematicBreak(_) => doc.items.push(MdItem::Rule),
                other => {
                    if let Some(ch) = other.children() {
                        walk(ch, doc, depth);
                    }
                }
            }
        }
    }

    let mut doc = MdDoc {
        items: Vec::new(),
        blocks: Vec::new(),
        block_srcs: Vec::new(),
    };
    if let Ok(ast) = markdown::to_mdast(md, &markdown::ParseOptions::default()) {
        if let Some(children) = ast.children() {
            walk(children, &mut doc, 0);
        }
    }
    doc
}

/// Susun potongan-potongan inline menjadi satu LayoutJob bergaya
/// (tebal/miring/kode/tautan), memakai warna tema aktif.
fn runs_job(ui: &egui::Ui, runs: &[Run], size: f32, all_strong: bool) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    for r in runs {
        let mut fmt = TextFormat {
            font_id: if r.code {
                FontId::monospace(size * 0.92)
            } else {
                FontId::proportional(size)
            },
            color: if r.strong || all_strong {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().text_color()
            },
            italics: r.em,
            ..Default::default()
        };
        if r.code {
            fmt.background = ui.visuals().code_bg_color;
        }
        if r.link {
            fmt.color = ui.visuals().hyperlink_color;
            fmt.underline = Stroke::new(1.0, ui.visuals().hyperlink_color);
        }
        job.append(&r.text, 0.0, fmt);
    }
    job
}

/// Gambar satu elemen dokumen Markdown. Klik "edit sebagai tab" pada
/// diagram menulis indeks bloknya ke `open_req`.
fn draw_md_item(ui: &mut egui::Ui, item: &MdItem, mdoc: &MdDoc, open_req: &mut Option<usize>) {
    match item {
        MdItem::Heading(depth, runs) => {
            let size = [24.0, 20.0, 17.0, 15.0, 14.0, 13.5][(*depth as usize - 1).min(5)];
            ui.add_space(if *depth <= 2 { 14.0 } else { 10.0 });
            ui.label(runs_job(ui, runs, size, true));
            if *depth <= 2 {
                ui.separator();
            }
            ui.add_space(4.0);
        }
        MdItem::Para(runs) => {
            ui.label(runs_job(ui, runs, 14.0, false));
            ui.add_space(8.0);
        }
        MdItem::Bullet(depth, num, runs) => {
            ui.horizontal_top(|ui| {
                ui.add_space(10.0 + *depth as f32 * 18.0);
                let marker = match num {
                    Some(k) => format!("{k}."),
                    None => "•".to_string(),
                };
                ui.label(egui::RichText::new(marker).size(14.0));
                ui.label(runs_job(ui, runs, 14.0, false));
            });
            ui.add_space(3.0);
        }
        MdItem::Quote(runs) => {
            egui::Frame::none()
                .fill(ui.visuals().faint_bg_color)
                .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                .rounding(4.0)
                .show(ui, |ui| {
                    ui.label(runs_job(ui, runs, 14.0, false));
                });
            ui.add_space(8.0);
        }
        MdItem::CodeBlock(text) => {
            egui::Frame::none()
                .fill(ui.visuals().code_bg_color)
                .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                .rounding(6.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .monospace()
                            .size(12.5)
                            .color(ui.visuals().text_color()),
                    );
                });
            ui.add_space(8.0);
        }
        MdItem::Rule => {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
        }
        MdItem::Diagram(i) => {
            match &mdoc.blocks[*i] {
                BlockView::Ok(d) if d.width > 0.0 && d.height > 0.0 => {
                    // Skala pas-lebar (tak pernah diperbesar >1).
                    let avail = ui.available_width().min(820.0);
                    let scale = ((avail - 24.0) / d.width as f32).min(1.0);
                    let size = Vec2::new(
                        d.width as f32 * scale + 24.0,
                        d.height as f32 * scale + 24.0,
                    );
                    let (resp, painter) =
                        ui.allocate_painter(size, egui::Sense::hover());
                    painter.rect(
                        resp.rect,
                        8.0,
                        Color32::WHITE,
                        Stroke::new(1.0, LABEL_BORDER),
                    );
                    let origin = resp.rect.min + Vec2::splat(12.0);
                    let ts = move |x: f64, y: f64| {
                        origin + Vec2::new(x as f32, y as f32) * scale
                    };
                    paint_diagram(&painter, &d.refs(), None, &ts, scale, false);
                }
                BlockView::Ok(_) => {
                    ui.weak("(diagram kosong)");
                }
                BlockView::Err(e) => {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(0x3a, 0x22, 0x24))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                        .rounding(6.0)
                        .show(ui, |ui| {
                            ui.colored_label(
                                Color32::from_rgb(0xe0, 0x5a, 0x5a),
                                format!("blok mermaid #{}: {}", i + 1, e),
                            );
                        });
                }
            }
            ui.horizontal(|ui| {
                ui.weak(format!("diagram #{}", i + 1));
                if ui.small_button("edit sebagai tab").clicked() {
                    *open_req = Some(*i);
                }
            });
            ui.add_space(10.0);
        }
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
    p.text(
        c,
        Align2::CENTER_CENTER,
        &n.label,
        zfont(14.0, zoom),
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

/// Blok ```mermaid di sebuah dokumen Markdown, diekstrak lewat AST
/// crate `markdown` (bukan regex — fence bertilde, di dalam quote,
/// dan info-string tetap terdeteksi benar). Tiap entri: isi blok +
/// rentang byte SELURUH fence (pembuka s/d penutup) di sumber.
fn mermaid_blocks(md: &str) -> Vec<(String, std::ops::Range<usize>)> {
    fn walk(node: &markdown::mdast::Node, out: &mut Vec<(String, std::ops::Range<usize>)>) {
        if let markdown::mdast::Node::Code(c) = node {
            let lang = c.lang.as_deref().unwrap_or("");
            if lang.eq_ignore_ascii_case("mermaid") || lang.eq_ignore_ascii_case("mmd") {
                if let Some(p) = &c.position {
                    out.push((c.value.clone(), p.start.offset..p.end.offset));
                }
            }
        }
        if let Some(children) = node.children() {
            for ch in children {
                walk(ch, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(ast) = markdown::to_mdast(md, &markdown::ParseOptions::default()) {
        walk(&ast, &mut out);
    }
    out
}

/// Ganti ISI blok mermaid ke-`index` di teks Markdown dengan `src`,
/// mempertahankan baris fence pembuka/penutup apa adanya (termasuk
/// info-string). Gagal (None) bila blok tak ditemukan lagi atau
/// fence-nya ter-indentasi (mis. di dalam list — belum didukung).
fn splice_md_block(md: &str, index: usize, src: &str) -> Option<String> {
    let (_, range) = mermaid_blocks(md).into_iter().nth(index)?;
    let block = &md[range.clone()];
    let open_len = block.find('\n')?;
    let close_start = block.rfind('\n')?;
    let (open, close) = (&block[..open_len], &block[close_start + 1..]);
    // Kedua baris harus benar-benar fence TANPA indentasi: fence di
    // dalam list menuntut re-indentasi isi (belum didukung), dan
    // fence tak tertutup di EOF membuat baris terakhir = konten.
    let fence = |s: &str| s.starts_with("```") || s.starts_with("~~~");
    if !fence(open) || !fence(close) {
        return None;
    }
    let mut out = String::with_capacity(md.len() + src.len());
    out.push_str(&md[..range.start]);
    out.push_str(open);
    out.push('\n');
    out.push_str(src.trim_end_matches('\n'));
    out.push('\n');
    out.push_str(close);
    out.push_str(&md[range.end..]);
    Some(out)
}

/// Scene kosong dengan ukuran kanvas — dipakai diagram statis
/// (pie/sequence) yang tak punya node yang bisa digeser.
fn blank_scene(width: f64, height: f64) -> Scene {
    Scene {
        nodes: Vec::new(),
        edges: Vec::new(),
        clusters: Vec::new(),
        width,
        height,
    }
}

/// Garis putus-putus lurus (egui tak punya dash bawaan) dalam
/// koordinat layar. `dash`/`gap` dalam piksel layar — pemanggil
/// mengalikan zoom supaya ritme dash cocok dengan `stroke-dasharray`
/// SVG pada level zoom berapa pun (paritas per elemen: lifeline 4/4,
/// divider 5/4, pesan 6/4 — temuan bug hunt).
fn dashed_line(p: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, dash: f32, gap: f32) {
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

/// Pie chart: judul, sektor, label persen, legenda. Cermin dari
/// `pie::to_svg`, memakai geometri `PieScene` yang sama. `labels` /
/// `empty` sudah di-precompute di `set_static_pie` (bebas alokasi).
fn draw_pie(
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
fn draw_sequence(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new(CONTOH.to_string(), None, Vec::new(), None)
    }

    #[test]
    fn write_to_reports_success_and_failure() {
        let mut a = app();
        let dir = std::env::temp_dir().join(format!("flowmaid-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("out.mmd");
        assert!(a.write_to(&f), "write to a real path must succeed");
        assert!(!a.dirty(), "successful save clears dirty");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), a.src);
        // A path whose parent is a file (not a dir) can't be written.
        let bad = f.join("nested.mmd");
        a.src.push_str("\nX-->Y");
        assert!(!a.write_to(&bad), "write to an invalid path must fail");
        assert!(a.dirty(), "failed save leaves the doc dirty");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hex_handles_css_forms_like_the_svg_renderer() {
        // Bug ditemukan bughunter: #f9f dulu jatuh ke GRAY di kanvas
        // padahal ekspor SVG merendernya magenta.
        assert_eq!(hex("#f9f"), Color32::from_rgb(0xff, 0x99, 0xff));
        assert_eq!(hex("#ff99ff"), Color32::from_rgb(0xff, 0x99, 0xff));
        assert_eq!(hex("red"), Color32::from_rgb(255, 0, 0));
        assert_eq!(hex(" #333 "), Color32::from_rgb(0x33, 0x33, 0x33));
        assert_eq!(hex("bukanwarna"), Color32::GRAY);
    }

    #[test]
    fn zoom_around_keeps_the_anchored_world_point_fixed() {
        let mut a = app();
        a.pan = Vec2::new(40.0, -20.0);
        let anchor = Vec2::new(300.0, 200.0);
        let world_before = (anchor - a.pan) / a.zoom;
        a.zoom_around(1.5, anchor);
        let world_after = (anchor - a.pan) / a.zoom;
        assert!((world_before - world_after).length() < 1e-3);
        assert!((a.zoom - 1.5).abs() < 1e-6);
        // Clamped at both ends.
        a.zoom_around(100.0, anchor);
        assert!(a.zoom <= MAX_ZOOM);
        a.zoom_around(1e-6, anchor);
        assert!(a.zoom >= MIN_ZOOM);
    }

    #[test]
    fn recent_files_dedupe_and_cap_at_eight() {
        let mut a = app();
        for i in 0..12 {
            a.push_recent(Path::new(&format!("/tmp/f{}.mmd", i % 10)));
        }
        assert!(a.recent.len() <= 8);
        // Re-opening an old file moves it to the front, no duplicate.
        a.push_recent(Path::new("/tmp/f5.mmd"));
        assert_eq!(a.recent[0], "/tmp/f5.mmd");
        assert_eq!(a.recent.iter().filter(|r| *r == "/tmp/f5.mmd").count(), 1);
    }

    #[test]
    fn dirty_tracks_divergence_from_saved_source() {
        let mut a = app();
        assert!(!a.dirty(), "fresh document starts clean");
        a.src.push_str("\nX --> Y\n");
        assert!(a.dirty());
        a.saved_src = a.src.clone();
        assert!(!a.dirty());
    }

    #[test]
    fn reparse_switches_between_flowchart_and_er_models() {
        let mut a = app();
        assert!(a.tables.is_empty(), "sample document is a flowchart");
        a.src = "erDiagram\nusers ||--o{ posts : writes".into();
        a.reparse();
        assert!(matches!(a.model, Model::Er(_)));
        assert_eq!(a.tables.len(), 2);
        assert_eq!(a.cards.len(), 1);
        // Positions preserved by key across an edit.
        let before = a.pos[0];
        a.pos[0] = (before.0 + 300.0, before.1);
        a.src = "erDiagram\nusers ||--o{ posts : writes\nposts }o--|| tags : has".into();
        a.reparse();
        assert_eq!(
            a.pos[0],
            (before.0 + 300.0, before.1),
            "users keeps its dragged spot"
        );
    }

    #[test]
    fn reparse_handles_class_model_and_clears_other_aux() {
        let mut a = app();
        a.src = "classDiagram\nAnimal <|-- Dog\nAnimal \"1\" o-- \"*\" Toy : owns".into();
        a.reparse();
        assert!(matches!(a.model, Model::Class(_)));
        assert_eq!(a.boxes.len(), 3, "Animal + Dog + Toy");
        assert_eq!(a.rels.len(), 2);
        assert!(a.tables.is_empty() && a.cards.is_empty(), "ER aux must be cleared");
        assert!(a.error.is_none());
        // Switching back to a flowchart clears the class aux again.
        a.src = "flowchart TD\nX --> Y".into();
        a.reparse();
        assert!(matches!(a.model, Model::Flow(_)));
        assert!(a.boxes.is_empty() && a.rels.is_empty(), "class aux must be cleared");
        // Export follows the active model without panicking.
        assert!(a.export_svg().contains("<svg"));
    }

    #[test]
    fn reparse_handles_static_pie_and_sequence_models() {
        let mut a = app();
        // Pie: geometry stored, no draggable positions, class/ER aux clear.
        a.src = "pie\n\"a\" : 3\n\"b\" : 1".into();
        a.reparse();
        assert!(matches!(a.model, Model::Pie(_)));
        assert!(a.pie.is_some() && a.seq.is_none());
        assert!(a.pos.is_empty(), "pie has no draggable nodes");
        assert_eq!(a.pie.as_ref().unwrap().slices.len(), 2);
        assert!(a.export_svg().contains("<svg"));

        // Sequence: geometry stored, pie aux cleared on switch.
        a.src = "sequenceDiagram\nA->>B: hi\nNote over A: n".into();
        a.reparse();
        assert!(matches!(a.model, Model::Sequence(_)));
        assert!(a.seq.is_some() && a.pie.is_none(), "pie aux must be cleared");
        assert!(!a.seq.as_ref().unwrap().messages.is_empty());
        assert!(a.export_svg().contains("</svg>"));

        // Back to a flowchart clears both static scenes.
        a.src = "flowchart TD\nX --> Y".into();
        a.reparse();
        assert!(a.pie.is_none() && a.seq.is_none());
    }

    #[test]
    fn tabs_open_switch_dedupe_and_close() {
        let dir = std::env::temp_dir().join(format!("flowmaid-tabs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mmd"), "flowchart TD\nA1-->A2").unwrap();
        std::fs::write(dir.join("b.mmd"), "pie\n\"x\" : 1").unwrap();

        let mut app = app();
        // Tab contoh yang belum disentuh ditimpa di tempat.
        app.open_path(dir.join("a.mmd"));
        assert_eq!(app.docs.len(), 1, "pristine untitled digantikan, bukan +tab");
        assert!(app.tab_title(0).starts_with("a.mmd"));

        // File kedua membuka TAB BARU dan aktif.
        app.open_path(dir.join("b.mmd"));
        assert_eq!(app.docs.len(), 2);
        assert_eq!(app.active, 1);
        assert!(matches!(app.model, Model::Pie(_)), "tab aktif = pie");

        // Membuka file yang sudah ada tabnya = pindah, bukan duplikat.
        app.open_path(dir.join("a.mmd"));
        assert_eq!(app.docs.len(), 2, "tak ada tab duplikat");
        assert_eq!(app.active, 0);
        assert!(matches!(app.model, Model::Flow(_)));

        // Geseran node bertahan saat bolak-balik tab.
        let dragged = (app.pos[0].0 + 300.0, app.pos[0].1);
        app.pos[0] = dragged;
        app.switch_to(1);
        app.switch_to(0);
        assert_eq!(app.pos[0], dragged, "posisi geser selamat lintas tab");

        // Tutup tab aktif yang bersih → tetangga termuat.
        app.request_close(0);
        assert!(app.pending.is_none(), "tab bersih tak butuh dialog");
        assert_eq!(app.docs.len(), 1);
        assert!(matches!(app.model, Model::Pie(_)), "tetangga (pie) jadi aktif");

        // Tab dirty ditahan dialog; tab terakhir tak pernah hilang.
        app.src.push_str("\n\"y\" : 2");
        app.request_close(0);
        assert!(matches!(app.pending, Some(Pending::CloseTab(0))));
        app.pending = None;
        app.perform(Pending::CloseTab(0)); // "Buang perubahan"
        assert_eq!(app.docs.len(), 1, "tab terakhir diganti dokumen baru");
        assert!(app.path.is_none() && !app.dirty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn markdown_blocks_extract_open_as_tabs_and_save_back() {
        let md = "# Judul\n\nteks pembuka\n\n```mermaid\nflowchart TD\nA-->B\n```\n\n\
                  paragraf tengah\n\n```js\nconsole.log(1)\n```\n\n\
                  ~~~mermaid\npie\n\"x\" : 1\n~~~\n\npenutup\n";
        // Ekstraksi: dua blok mermaid, fence js dilewati.
        let blocks = mermaid_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].0.starts_with("flowchart TD"));
        assert!(blocks[1].0.starts_with("pie"));

        // Splice mengganti isi blok #2 tanpa menyentuh sekitarnya.
        let out = splice_md_block(md, 1, "pie\n\"y\" : 9").unwrap();
        assert!(out.contains("~~~mermaid\npie\n\"y\" : 9\n~~~"));
        assert!(out.contains("console.log(1)") && out.contains("penutup"));
        assert!(out.contains("A-->B"), "blok #1 tak tersentuh");

        // Alur app: buka .md → SATU tab dokumen ter-render.
        let dir = std::env::temp_dir().join(format!("flowmaid-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mdpath = dir.join("doc.md");
        std::fs::write(&mdpath, md).unwrap();
        let mut a = app();
        a.open_path(mdpath.clone());
        assert_eq!(a.docs.len(), 1, "satu file md = satu tab dokumen");
        assert!(a.path_is_markdown());
        let m = a.mdoc.as_ref().expect("mode dokumen aktif");
        assert_eq!(m.blocks.len(), 2, "dua diagram inline");
        assert!(m.items.iter().any(|i| matches!(i, MdItem::Heading(1, _))));
        assert!(m.items.iter().any(|i| matches!(i, MdItem::CodeBlock(_))), "fence js jadi code block");
        assert!(matches!(m.blocks[0], BlockView::Ok(_)));
        assert!(a.tab_title(0).starts_with("doc.md"));

        // "edit sebagai tab": blok #1 jadi tab diagram tersendiri.
        let src0 = m.block_srcs[0].clone();
        a.open_md_block(&mdpath.clone(), 0, src0);
        assert_eq!(a.docs.len(), 2);
        assert!(a.tab_title(1).starts_with("doc.md #1"));
        assert!(matches!(a.model, Model::Flow(_)));

        // Edit lalu simpan → menulis balik ke fence di file induk.
        a.src = "flowchart TD\nA-->C".into();
        assert!(a.save_doc(), "simpan blok md harus sukses");
        let on_disk = std::fs::read_to_string(&mdpath).unwrap();
        assert!(on_disk.contains("```mermaid\nflowchart TD\nA-->C\n```"));
        assert!(on_disk.contains("# Judul") && on_disk.contains("~~~mermaid"));
        assert!(!a.dirty());

        // Dedupe dua arah: blok yang sama & file md yang sama.
        a.open_md_block(&mdpath.clone(), 0, String::new());
        assert_eq!(a.docs.len(), 2, "blok sudah terbuka → pindah saja");
        a.open_path(mdpath);
        assert_eq!(a.docs.len(), 2, "dokumen sudah terbuka → pindah saja");
        assert!(a.mdoc.is_some(), "kembali ke tab dokumen");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn explorer_sees_file_the_app_just_saved() {
        // Bug hunt: write_to tidak meng-invalidasi dir_cache, jadi
        // file hasil Simpan-Sebagai tak pernah muncul di explorer
        // sampai "Segarkan" manual.
        let mut a = app();
        let dir = std::env::temp_dir().join(format!("flowmaid-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mmd"), "A-->B").unwrap();
        let names = |l: &Rc<Result<Vec<TreeEntry>, String>>| -> Vec<String> {
            l.as_ref().as_ref().unwrap().iter().map(|t| t.name.clone()).collect()
        };
        assert_eq!(names(&a.listing(&dir)), ["a.mmd"], "cache primed");
        assert!(a.write_to(&dir.join("b.mmd")), "save into the cached folder");
        assert!(
            names(&a.listing(&dir)).contains(&"b.mmd".to_string()),
            "explorer must show the file the app just saved"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
