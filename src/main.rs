//! flowmaid desktop — editor diagram interaktif di atas engine flowmaid.
//!
//! - Multi-file : tab dokumen ala editor — klik file membuka tab baru,
//!   Cmd+W menutup, tab dirty ditahan dialog, sesi tab dipulihkan
//! - Markdown   : buka .md → dokumen ter-render via engine `markmaid`
//!   (parse+layout → DocScene, dilukis native); tiap blok ```mermaid
//!   bisa dibuka jadi tab, Simpan menulis balik ke dalam fence-nya
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

mod app;
mod md;
mod model;
mod paint;
mod paint_static;
mod theme;
mod ui;

#[cfg(test)]
mod tests;

use crate::app::App;
use crate::model::CONTOH;
use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;

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
            // Folder explorer yang terbuka di sesi sebelumnya.
            let open_dirs: HashSet<PathBuf> = cc
                .storage
                .and_then(|s| s.get_string("open_dirs"))
                .map(|s| {
                    s.lines()
                        .filter(|l| !l.is_empty())
                        .map(PathBuf::from)
                        .filter(|p| p.is_dir())
                        .collect()
                })
                .unwrap_or_default();
            let mut app = App::new(CONTOH.to_string(), None, recent, workspace);
            app.open_dirs = open_dirs;
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
