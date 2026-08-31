//! Tipe data aplikasi: dokumen valid terakhir (`Model`), state satu
//! tab (`Doc`), dan entri pohon explorer.

use crate::md::MdView;
use crate::paint::blank_scene;
use eframe::egui::Vec2;
use flowmaid::class::{ClassBox, RelStyle};
use flowmaid::er::ErTable;
use flowmaid::journey::JourneyScene;
use flowmaid::mindmap::MindScene;
use flowmaid::model::{
    Architecture, Card, ClassDiagram, ErDiagram, GitGraph, Graph, Journey, Mindmap, PieChart,
    SequenceDiagram,
};
use flowmaid::pie::PieScene;
use flowmaid::scene::Scene;
use flowmaid::seq::SeqScene;
use std::path::PathBuf;

pub(crate) const CONTOH: &str = "%% Geser node dengan mouse, atau edit teks ini.\n%% Warna kustom: style / classDef / ::: ala mermaid.\nflowchart TD\n    A([Mulai]) --> B[Baca input]\n    B --> C{Valid?}\n    C -->|ya| D[Proses data]\n    C -->|tidak| E[Tampilkan error]\n    E --> B\n    D ==> F((Selesai))\n    classDef bahaya fill:#ffe3e3,stroke:#e03131,color:#c92a2a\n    E:::bahaya\n";

/// Aksi yang bisa membuang perubahan; ditunda ke dialog konfirmasi
/// bila dokumen yang bersangkutan sedang dirty. Sejak ada tab,
/// membuka file tak pernah membuang apa pun (selalu jadi tab baru) —
/// hanya menutup tab yang butuh konfirmasi.
pub(crate) enum Pending {
    CloseTab(usize),
}

/// Tab area utama: pratinjau, terbelah, atau editor teks.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum View {
    Preview,
    Split,
    Code,
}

/// Dokumen valid terakhir. Flow/ER/class punya node yang bisa
/// digeser; pie & sequence statis (digambar apa adanya).
pub(crate) enum Model {
    Flow(Graph),
    Er(ErDiagram),
    Class(ClassDiagram),
    Pie(PieChart),
    Sequence(SequenceDiagram),
    Mindmap(Mindmap),
    Journey(Journey),
    GitGraph(GitGraph),
    Architecture(Architecture),
}

impl Model {
    /// Kunci identitas node/entitas/class ke-i, untuk mempertahankan
    /// posisi geseran saat teks diedit. Pie/sequence tak punya node yang
    /// bisa digeser (kosong); mindmap pakai teks node sebagai identitas.
    pub(crate) fn keys(&self) -> Vec<&str> {
        match self {
            Model::Flow(g) => g.nodes.iter().map(|n| n.id.as_str()).collect(),
            Model::Er(d) => d.entities.iter().map(|e| e.name.as_str()).collect(),
            Model::Class(d) => d.classes.iter().map(|c| c.name.as_str()).collect(),
            Model::Mindmap(m) => m.nodes.iter().map(|n| n.text.as_str()).collect(),
            Model::Pie(_) | Model::Sequence(_) | Model::Journey(_) => Vec::new(),
            Model::GitGraph(_) | Model::Architecture(_) => Vec::new(),
        }
    }
}

/// Satu entri hasil listing folder. Nama tampilan & jenis (folder /
/// file) di-precompute saat cache diisi, supaya draw_tree tidak
/// melakukan syscall stat() maupun alokasi String per entri per frame.
pub(crate) struct TreeEntry {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
}

/// Dokumen ini berasal dari satu blok ```mermaid di dalam file
/// Markdown: `path` = file .md induk, `index` = blok mermaid ke-n
/// (0-based). Menyimpan berarti menulis balik KE DALAM fence-nya.
#[derive(Clone)]
pub(crate) struct MdHost {
    pub(crate) path: PathBuf,
    pub(crate) index: usize,
}

/// State lengkap satu dokumen (satu tab). Dokumen AKTIF tinggal di
/// field-field `App` (kode gambar/editor tak perlu berubah); struct
/// ini memarkir tab non-aktif, di-swap saat pindah tab. Entri milik
/// tab aktif di `App::docs` adalah cangkang kosong.
pub(crate) struct Doc {
    pub(crate) src: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) md_host: Option<MdHost>,
    pub(crate) saved_src: String,
    pub(crate) model: Model,
    pub(crate) pos: Vec<(f64, f64)>,
    pub(crate) scn: Scene,
    pub(crate) tables: Vec<ErTable>,
    pub(crate) cards: Vec<(Card, Card)>,
    pub(crate) boxes: Vec<ClassBox>,
    pub(crate) rels: Vec<RelStyle>,
    pub(crate) pie: Option<PieScene>,
    pub(crate) seq: Option<SeqScene>,
    pub(crate) mind: Option<MindScene>,
    pub(crate) journey: Option<JourneyScene>,
    pub(crate) pie_labels: Vec<Option<String>>,
    pub(crate) pie_empty: bool,
    pub(crate) seq_labels: Vec<String>,
    /// Some = tab ini DOKUMEN Markdown ter-render (bukan diagram).
    pub(crate) mdoc: Option<MdView>,
    pub(crate) error: Option<String>,
    pub(crate) zoom: f32,
    pub(crate) pan: Vec2,
    /// Geometri scene() otomatis terakhir + posisinya — jangkar
    /// route_partial: drag jadi operasi lokal (edge yang tak tersentuh
    /// mempertahankan kualitas engine). None = belum ada / graph beda.
    pub(crate) auto_scn: Option<Scene>,
    pub(crate) auto_pos: Vec<(f64, f64)>,
    /// Pernah ada node yang digeser sejak buka/tata-ulang? Selama
    /// false, reparse menampilkan geometri `scene()` engine utuh
    /// (routing channel + slot label); `route()` baru dipakai untuk
    /// mempertahankan posisi begitu user benar-benar menggeser.
    pub(crate) dragged: bool,
}

impl Doc {
    /// Cangkang kosong — placeholder untuk slot tab aktif.
    pub(crate) fn empty() -> Doc {
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
            mind: None,
            journey: None,
            pie_labels: Vec::new(),
            pie_empty: false,
            seq_labels: Vec::new(),
            mdoc: None,
            error: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            auto_scn: None,
            auto_pos: Vec::new(),
            dragged: false,
        }
    }
}
