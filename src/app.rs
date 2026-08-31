//! State aplikasi & logikanya: parse/route/layout ulang, tab dokumen,
//! I/O file (buka, simpan, splice blok Markdown), dan cache explorer.

use crate::md::{build_mdview, MdView};
use crate::model::{Doc, MdHost, Model, Pending, TreeEntry, View, CONTOH};
use crate::paint::blank_scene;
use eframe::egui::Vec2;
use flowmaid::class::{self, ClassBox, RelStyle};
use flowmaid::er::{self, ErTable};
use flowmaid::journey::{self, JourneyScene};
use flowmaid::mindmap::{self, MindScene};
use flowmaid::model::{Card, Graph};
use flowmaid::pie::{self, PieScene};
use flowmaid::scene::{route, scene, to_svg, Scene};
use flowmaid::seq::{self, SeqScene};
use flowmaid::Document;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub(crate) const MIN_ZOOM: f32 = 0.2;
pub(crate) const MAX_ZOOM: f32 = 4.0;

pub(crate) struct App {
    // Tab dokumen: docs[active] = cangkang; state aslinya ada di
    // field src/path/model/... di bawah (dokumen aktif).
    pub(crate) docs: Vec<Doc>,
    pub(crate) active: usize,
    pub(crate) src: String,
    pub(crate) path: Option<PathBuf>, // file yang sedang dibuka (None = belum disimpan)
    pub(crate) md_host: Option<MdHost>, // Some = dokumen ini blok mermaid di file .md
    pub(crate) saved_src: String,     // isi terakhir yang tersimpan, untuk deteksi dirty
    pub(crate) recent: Vec<String>,   // file terakhir dibuka, terbaru di depan
    pub(crate) workspace: Option<PathBuf>, // folder explorer ala VSCode (panel kiri)
    // Cache isi tiap folder yang sudah dibaca — explorer tak lagi
    // menyentuh filesystem tiap frame. Ok(entries) sudah ter-filter
    // & terurut; Err = pesan gagal baca (mis. folder tercabut).
    // Rc: draw_tree meminjam listing tanpa deep-clone per frame.
    pub(crate) dir_cache: HashMap<PathBuf, Rc<Result<Vec<TreeEntry>, String>>>,
    // Folder explorer yang sedang terbuka (tak ada di set = terlipat);
    // dipersist antar-sesi seperti VSCode. Header section melipat
    // seluruh pohon lewat tree_root_open.
    pub(crate) open_dirs: HashSet<PathBuf>,
    pub(crate) tree_root_open: bool,
    pub(crate) view: View,               // tab aktif: Preview / Code
    pub(crate) pending: Option<Pending>, // aksi menunggu konfirmasi buang-perubahan
    pub(crate) last_title: String,
    // Kunci perubahan judul jendela (None = belum pernah dihitung).
    pub(crate) last_dirty: Option<bool>,
    pub(crate) last_titled_path: Option<PathBuf>,
    pub(crate) last_titled_tab: usize,
    pub(crate) model: Model,             // dokumen valid terakhir
    pub(crate) pos: Vec<(f64, f64)>,     // posisi node/entitas, milik aplikasi (bisa digeser)
    pub(crate) scn: Scene,               // geometri terkini untuk digambar
    pub(crate) tables: Vec<ErTable>,     // data tabel ER (kosong untuk flowchart)
    pub(crate) cards: Vec<(Card, Card)>, // kardinalitas per relasi ER, sejajar scn.edges
    pub(crate) boxes: Vec<ClassBox>,     // data box class (kosong untuk non-class)
    pub(crate) rels: Vec<RelStyle>,      // gaya/kardinalitas relasi class, sejajar scn.edges
    pub(crate) pie: Option<PieScene>,    // geometri pie (Some hanya untuk Model::Pie)
    pub(crate) seq: Option<SeqScene>,    // geometri sequence (Some hanya untuk Model::Sequence)
    pub(crate) mind: Option<MindScene>,  // geometri mindmap (Some hanya untuk Model::Mindmap)
    pub(crate) journey: Option<JourneyScene>, // geometri journey (Some hanya untuk Model::Journey)
    // Precompute tampilan diagram statis (hindari format! per frame):
    pub(crate) pie_labels: Vec<Option<String>>, // "NN%" per slice; None = terlalu tipis
    pub(crate) pie_empty: bool,                 // total 0 → outline saja
    pub(crate) seq_labels: Vec<String>,         // label pesan final ("N. teks" / teks)
    pub(crate) mdoc: Option<MdView>,             // Some = tab dokumen Markdown ter-render
    pub(crate) error: Option<String>,
    pub(crate) status: String,
    pub(crate) zoom: f32,         // faktor zoom kanvas (1.0 = 100%)
    pub(crate) pan: Vec2,         // geseran kanvas, piksel layar
    pub(crate) canvas_size: Vec2, // ukuran kanvas frame terakhir (jangkar zoom via tombol)
    pub(crate) auto_scn: Option<Scene>,  // scene() otomatis terakhir (lihat Doc::auto_scn)
    pub(crate) auto_pos: Vec<(f64, f64)>,
    pub(crate) dragged: bool,     // dokumen aktif: sudah ada node yang digeser (lihat Doc::dragged)
}

impl App {
    pub(crate) fn new(
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
            open_dirs: HashSet::new(),
            tree_root_open: true,
            view: View::Split,
            pending: None,
            last_title: String::new(),
            last_dirty: None,
            last_titled_path: None,
            last_titled_tab: usize::MAX,
            model: Model::Flow(Graph::default()),
            pos: Vec::new(),
            scn: blank_scene(0.0, 0.0),
            tables: Vec::new(),
            cards: Vec::new(),
            boxes: Vec::new(),
            rels: Vec::new(),
            pie: None,
            seq: None,
            mind: None,
            journey: None,
            pie_labels: Vec::new(),
            pie_empty: false,
            seq_labels: Vec::new(),
            mdoc: None,
            error: None,
            status: "geser node dengan mouse".into(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            canvas_size: Vec2::ZERO,
            auto_scn: None,
            auto_pos: Vec::new(),
            dragged: false,
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
    pub(crate) fn reparse(&mut self) {
        // Dokumen Markdown dirender sebagai DOKUMEN (heading, teks,
        // diagram inline) — bukan diparse sebagai diagram.
        if self.path_is_markdown() {
            self.mdoc = Some(build_mdview(&self.src));
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
                // Posisi lama hanya dipertahankan bila user pernah
                // menggeser node; tanpa drag, tiap reparse mengikuti
                // auto-layout engine sepenuhnya (geometri scene() utuh,
                // seperti mermaid live) alih-alih dirutekan ulang.
                let old: HashMap<&str, (f64, f64)> = if self.dragged {
                    prev.keys()
                        .into_iter()
                        .zip(prev_pos.iter().copied())
                        .collect()
                } else {
                    HashMap::new()
                };
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
                    Document::Journey(d) => {
                        self.model = Model::Journey(d);
                    }
                    // Mindmap nodes ARE draggable: seed from the radial
                    // auto-layout, keeping any node whose text (its key)
                    // survived the edit at its dragged position. Only a
                    // UNIQUELY-named node is preserved — duplicate labels
                    // are indistinguishable by key, so they fall back to
                    // auto-layout instead of all stacking on one saved
                    // position.
                    Document::Mindmap(d) => {
                        let mut counts: HashMap<&str, usize> = HashMap::new();
                        for n in &d.nodes {
                            *counts.entry(n.text.as_str()).or_insert(0) += 1;
                        }
                        let mut auto = None;
                        self.pos = d
                            .nodes
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                let unique = counts[n.text.as_str()] == 1;
                                unique
                                    .then(|| old.get(n.text.as_str()).copied())
                                    .flatten()
                                    .unwrap_or_else(|| {
                                        let a: &MindScene =
                                            auto.get_or_insert_with(|| mindmap::scene(&d));
                                        (a.nodes[i].cx(), a.nodes[i].cy())
                                    })
                            })
                            .collect();
                        self.model = Model::Mindmap(d);
                    }
                    // Static, auto-laid-out: no per-node drag state.
                    Document::GitGraph(d) => {
                        self.model = Model::GitGraph(d);
                    }
                    Document::Architecture(d) => {
                        self.model = Model::Architecture(d);
                    }
                }
                if self.dragged {
                    // Sumber berubah saat posisi dipertahankan — jangkar
                    // scene() lama tak lagi sejajar; route_partial akan
                    // fallback ke route() sampai tata-ulang berikutnya.
                    self.auto_scn = None;
                    self.auto_pos.clear();
                    self.reroute();
                } else {
                    // Semua posisi dari auto-layout — pakai geometri
                    // scene() langsung (edge lewat channel + label di
                    // slotnya), tanpa mengubah zoom/pan user.
                    self.apply_auto_scene();
                }
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub(crate) fn reroute(&mut self) {
        self.clear_aux();
        match &self.model {
            // Drag = operasi lokal: edge yang tak menyentuh node yang
            // bergeser mempertahankan geometri scene() dari jangkar
            // auto_scn; fallback route() penuh saat jangkar tak ada.
            Model::Flow(g) => {
                self.scn = match &self.auto_scn {
                    Some(base) if base.nodes.len() == g.nodes.len() => {
                        flowmaid::scene::route_partial(g, &self.pos, base, &self.auto_pos)
                    }
                    _ => route(g, &self.pos),
                }
            }
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
            Model::Journey(d) => self.set_static_journey(journey::scene(d)),
            // Mindmap: re-route connectors from the (draggable) centres.
            Model::Mindmap(d) => {
                let ms = mindmap::route(d, &self.pos);
                self.scn = blank_scene(ms.width, ms.height);
                self.mind = Some(ms);
            }
            Model::GitGraph(d) => {
                let gs = flowmaid::gitgraph::scene(d);
                self.scn = gs.scene;
            }
            Model::Architecture(d) => {
                let as_ = flowmaid::architecture::scene(d);
                self.scn = as_.scene;
            }
        }
    }

    /// Kembali ke tata letak otomatis engine (tombol "Tata ulang"):
    /// terapkan geometri auto, pulihkan pandangan, dan lupakan drag.
    pub(crate) fn autolayout(&mut self) {
        self.apply_auto_scene();
        self.dragged = false;
        self.reset_view();
    }

    /// Terapkan geometri `scene()` engine apa adanya untuk model
    /// aktif — routing channel, slot label, dan kotak cluster utuh —
    /// lalu selaraskan `pos` ke pusat node hasil layout. Dipakai
    /// tombol tata-ulang DAN reparse selama belum ada drag.
    fn apply_auto_scene(&mut self) {
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
            Model::Journey(d) => self.set_static_journey(journey::scene(d)),
            Model::Mindmap(d) => {
                let ms = mindmap::scene(d);
                self.pos = ms.nodes.iter().map(|n| (n.cx(), n.cy())).collect();
                self.scn = blank_scene(ms.width, ms.height);
                self.mind = Some(ms);
            }
            Model::GitGraph(d) => {
                let gs = flowmaid::gitgraph::scene(d);
                self.scn = gs.scene;
            }
            Model::Architecture(d) => {
                let as_ = flowmaid::architecture::scene(d);
                self.scn = as_.scene;
            }
        }
        // Scene-based types re-seed positions from the laid-out nodes;
        // the mindmap arm already set its own (radial) centres above.
        if !matches!(self.model, Model::Mindmap(_)) {
            self.pos = self.scn.nodes.iter().map(|n| (n.x, n.y)).collect();
        }
        // Jangkar route_partial — hanya bermakna untuk flowchart/state.
        if matches!(self.model, Model::Flow(_)) {
            self.auto_scn = Some(self.scn.clone());
            self.auto_pos = self.pos.clone();
        } else {
            self.auto_scn = None;
            self.auto_pos.clear();
        }
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
        self.mind = None;
        self.journey = None;
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

    /// Simpan geometri journey statis; `scn` memegang ukuran kanvas.
    fn set_static_journey(&mut self, js: JourneyScene) {
        self.scn = blank_scene(js.width, js.height);
        self.journey = Some(js);
    }

    /// SVG dari susunan saat ini (termasuk hasil geseran).
    pub(crate) fn export_svg(&self) -> String {
        match &self.model {
            Model::Flow(_) => to_svg(&self.scn),
            Model::Er(d) => er::to_svg(&er::route(d, &self.pos)),
            Model::Class(d) => class::to_svg(&class::route(d, &self.pos)),
            Model::Pie(d) => pie::to_svg(&pie::scene(d)),
            Model::Sequence(d) => seq::to_svg(&seq::scene(d)),
            // Reflect any dragging via route() over the live positions.
            Model::Mindmap(d) => mindmap::to_svg(&mindmap::route(d, &self.pos)),
            Model::Journey(d) => journey::to_svg(&journey::scene(d)),
            Model::GitGraph(d) => flowmaid::gitgraph::to_svg(&flowmaid::gitgraph::scene(d)),
            Model::Architecture(d) => {
                flowmaid::architecture::to_svg(&flowmaid::architecture::scene(d))
            }
        }
    }

    /// Zoom dengan jangkar tetap (koordinat lokal kanvas): titik dunia
    /// yang berada di bawah jangkar tidak bergeser di layar.
    pub(crate) fn zoom_around(&mut self, factor: f32, anchor: Vec2) {
        let target = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = target / self.zoom;
        self.pan = anchor - (anchor - self.pan) * k;
        self.zoom = target;
    }

    pub(crate) fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }

    pub(crate) fn dirty(&self) -> bool {
        self.src != self.saved_src
    }

    pub(crate) fn push_recent(&mut self, p: &Path) {
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
        d.mind = self.mind.take();
        d.journey = self.journey.take();
        d.pie_labels = std::mem::take(&mut self.pie_labels);
        d.pie_empty = self.pie_empty;
        d.seq_labels = std::mem::take(&mut self.seq_labels);
        d.mdoc = self.mdoc.take();
        d.error = self.error.take();
        d.zoom = self.zoom;
        d.pan = self.pan;
        d.auto_scn = self.auto_scn.take();
        d.auto_pos = std::mem::take(&mut self.auto_pos);
        d.dragged = self.dragged;
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
        self.mind = d.mind.take();
        self.journey = d.journey.take();
        self.pie_labels = std::mem::take(&mut d.pie_labels);
        self.pie_empty = d.pie_empty;
        self.seq_labels = std::mem::take(&mut d.seq_labels);
        self.mdoc = d.mdoc.take();
        self.error = d.error.take();
        self.zoom = d.zoom;
        self.pan = d.pan;
        self.auto_scn = d.auto_scn.take();
        self.auto_pos = std::mem::take(&mut d.auto_pos);
        self.dragged = d.dragged;
    }

    pub(crate) fn switch_to(&mut self, i: usize) {
        if i != self.active && i < self.docs.len() {
            self.park();
            self.load(i);
        }
    }

    /// Judul tab ke-`i` (nama file / blok md / "tanpa judul") — tab
    /// aktif dibaca dari field live, bukan dari cangkangnya.
    pub(crate) fn tab_title(&self, i: usize) -> String {
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

    /// Dokumen aktif adalah file Markdown? (mode dokumen ter-render)
    pub(crate) fn path_is_markdown(&self) -> bool {
        self.path
            .as_ref()
            .and_then(|p| p.extension())
            .map(|e| {
                let e = e.to_string_lossy().to_lowercase();
                e == "md" || e == "markdown"
            })
            .unwrap_or(false)
    }

    pub(crate) fn open_path(&mut self, p: PathBuf) {
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
    pub(crate) fn open_md_block(&mut self, host: &Path, index: usize, src: String) {
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
    pub(crate) fn new_file(&mut self) {
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
    pub(crate) fn request_close(&mut self, i: usize) {
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
    pub(crate) fn save_doc(&mut self) -> bool {
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
        let Some(next) = markmaid::blocks::splice(&md, host.index, &self.src) else {
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
    pub(crate) fn save_as(&mut self) -> bool {
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

    pub(crate) fn write_to(&mut self, p: &Path) -> bool {
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

    pub(crate) fn export_svg_file(&mut self) {
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
    pub(crate) fn open_folder_dialog(&mut self) {
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
    pub(crate) fn listing(&mut self, dir: &Path) -> Rc<Result<Vec<TreeEntry>, String>> {
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

    pub(crate) fn perform(&mut self, act: Pending) {
        match act {
            Pending::CloseTab(i) => self.close_tab(i),
        }
    }

    /// Dialog buka file — hasilnya jadi tab (tak ada yang terbuang).
    pub(crate) fn open_dialog(&mut self) {
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
