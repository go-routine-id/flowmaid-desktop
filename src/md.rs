//! Markdown: dokumen ter-render dengan diagram inline — layout via
//! `markmaid` (metrik font egui asli) lalu dilukis native.

use crate::paint::{blank_scene, paint_embedded};
use crate::theme::role;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use flowmaid::scene::Scene;

/// Dokumen Markdown ter-layout oleh **markmaid**: geometri absolut
/// (`DocScene`) yang tinggal dilukis — teks, kotak, garis, gambar, dan
/// tiap blok ```mermaid sebagai diagram flowmaid yang tertanam. Di-
/// relayout hanya saat lebar panel berubah; sumber tiap blok mermaid
/// disimpan agar bisa dibuka "sebagai tab" untuk disunting.
pub(crate) struct MdView {
    pub(crate) doc: markmaid::Doc,
    pub(crate) scene: markmaid::DocScene,
    /// Lebar konten (px) tempat `scene` terakhir dihitung; 0 = belum.
    pub(crate) laid_width: f32,
    /// Sumber tiap blok mermaid, urut dokumen — indeksnya sejajar
    /// dengan urutan diagram di `scene` dan dengan `blocks::splice`.
    pub(crate) block_srcs: Vec<String>,
}

impl MdView {
    /// Hitung ulang geometri untuk lebar konten `width` (px).
    ///
    /// `fonts` adalah snapshot font egui yang AKTUAL dipakai melukis.
    /// markmaid menerima metrik lebar teks dari konsumer (bukan
    /// estimasi built-in) supaya wrapping, posisi, dan chip `inline
    /// code` sejajar persis dengan glyph yang benar-benar digambar —
    /// ini yang memperbaiki layout "melenceng" untuk dokumen kompleks.
    pub(crate) fn relayout(&mut self, width: f32, fonts: &egui::text::Fonts) {
        // Klon murah (Arc) — dipindah ke closure yang harus 'static.
        let fonts = fonts.clone();
        let measure = markmaid::Measure::custom(
            move |text: &str, size: f64, mono: bool, em: bool| {
                use egui::text::{LayoutJob, TextFormat};
                let font_id = if mono {
                    egui::FontId::monospace(size as f32)
                } else {
                    egui::FontId::proportional(size as f32)
                };
                // Sama persis dengan `paint_docscene`: hanya `em`
                // (italic) yang mengubah lebar glyph — `strong` digambar
                // sebagai WARNA, bukan font tebal, jadi tak ikut diukur.
                let mut job = LayoutJob::default();
                job.append(
                    text,
                    0.0,
                    TextFormat {
                        font_id,
                        italics: em,
                        ..Default::default()
                    },
                );
                fonts.layout_job(job).size().x as f64
            },
        );
        let opts = markmaid::LayoutOptions {
            width: width as f64,
            base_size: 14.0,
            measure,
            table_overflow: markmaid::TableOverflow::Natural,
        };
        self.scene = markmaid::layout(&self.doc, &opts);
        self.laid_width = width;
    }

    /// Layout dengan metrik estimasi bawaan markmaid (tanpa font egui).
    /// Dipakai jalur SVG/HTML dan tes — geometri yang konsisten dengan
    /// writer bawaan, bukan dengan glyph egui di layar.
    #[cfg(test)]
    pub(crate) fn relayout_estimated(&mut self, width: f32) {
        let opts = markmaid::LayoutOptions {
            width: width as f64,
            base_size: 14.0,
            measure: markmaid::Measure::Estimated,
            table_overflow: markmaid::TableOverflow::Natural,
        };
        self.scene = markmaid::layout(&self.doc, &opts);
        self.laid_width = width;
    }
}

/// Parse Markdown lewat markmaid; geometri dihitung malas saat digambar
/// (butuh lebar panel). Sumber blok mermaid diambil dari scanner teks
/// mentah supaya indeksnya cocok dengan `blocks::splice` saat menyimpan.
pub(crate) fn build_mdview(md: &str) -> MdView {
    MdView {
        doc: markmaid::parse(md),
        scene: markmaid::DocScene::default(),
        laid_width: 0.0,
        block_srcs: markmaid::blocks::mermaid_blocks(md)
            .into_iter()
            .map(|(src, _)| src)
            .collect(),
    }
}

/// Test whether a link zone lies inside a table's natural scroll area.
fn link_in_zone(lz: &markmaid::LinkZone, tz: &markmaid::TableZone) -> bool {
    lz.x >= tz.x
        && lz.x + lz.w <= tz.x + tz.natural_w + 1e-6
        && lz.y >= tz.y
        && lz.y + lz.h <= tz.y + tz.h + 1e-6
}

/// Lukis satu item markmaid dengan origin layar yang diberikan.
#[allow(clippy::too_many_arguments)]
fn paint_markmaid_item(
    ui: &mut egui::Ui,
    view: &MdView,
    item: &markmaid::Item,
    origin: Pos2,
    vis: &egui::Visuals,
    blank: &Scene,
    open_req: &mut Option<usize>,
    diagram_ord: &mut usize,
) {
    use egui::text::{LayoutJob, TextFormat};
    use markmaid::ColorRole;

    let at = |x: f64, y: f64| origin + Vec2::new(x as f32, y as f32);
    match item {
        markmaid::Item::Rect(r) => {
            let rect = Rect::from_min_size(at(r.x, r.y), Vec2::new(r.w as f32, r.h as f32));
            let fill = r.fill.map_or(Color32::TRANSPARENT, |f| role(vis, f));
            let stroke = r.stroke.map_or(Stroke::NONE, |s| Stroke::new(1.0_f32, role(vis, s)));
            ui.painter().rect(rect, r.rounding as f32, fill, stroke);
        }
        markmaid::Item::Line(l) => {
            ui.painter().line_segment(
                [at(l.x1, l.y1), at(l.x2, l.y2)],
                Stroke::new(1.0_f32, role(vis, l.role)),
            );
        }
        markmaid::Item::Text(t) => {
            let color = role(vis, t.role);
            let font = if t.mono {
                FontId::monospace(t.size as f32)
            } else {
                FontId::proportional(t.size as f32)
            };
            let deco = |on: bool| if on { Stroke::new(1.0_f32, color) } else { Stroke::NONE };
            let mut job = LayoutJob::default();
            job.append(
                &t.text,
                0.0,
                TextFormat {
                    font_id: font,
                    color,
                    italics: t.em,
                    underline: deco(t.underline),
                    strikethrough: deco(t.strike),
                    ..Default::default()
                },
            );
            // markmaid y = puncak line box → jangkar kiri-atas.
            ui.painter().galley(at(t.x, t.y), ui.painter().layout_job(job), color);
        }
        markmaid::Item::Image(im) => {
            // Engine tak mendekode piksel: gambar jadi kotak
            // placeholder berbingkai dengan teks alt di tengah.
            let rect = Rect::from_min_size(at(im.x, im.y), Vec2::new(im.w as f32, im.h as f32));
            ui.painter().rect(
                rect,
                4.0,
                role(vis, ColorRole::CodeBg),
                Stroke::new(1.0_f32, role(vis, ColorRole::Border)),
            );
            let label = if im.alt.is_empty() {
                "▢ gambar".to_string()
            } else {
                format!("▢ {}", im.alt)
            };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(12.0),
                role(vis, ColorRole::Muted),
            );
        }
        markmaid::Item::Diagram(d) => {
            let sc = d.scale;
            let ts = |ex: f64, ey: f64| {
                origin + Vec2::new((d.x + ex * sc) as f32, (d.y + ey * sc) as f32)
            };
            paint_embedded(ui.painter(), &d.view, blank, &ts, sc as f32);
            // Diagram bisa diklik → buka blok sumbernya sebagai tab.
            let drect = Rect::from_min_size(
                at(d.x, d.y),
                Vec2::new((d.size.0 * sc) as f32, (d.size.1 * sc) as f32),
            );
            let resp = ui
                .interact(
                    drect,
                    egui::Id::new(("mmd-diagram", *diagram_ord)),
                    Sense::click(),
                )
                .on_hover_text("klik untuk sunting blok ini sebagai tab");
            if resp.hovered() {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
            }
            if resp.clicked() && *diagram_ord < view.block_srcs.len() {
                *open_req = Some(*diagram_ord);
            }
            *diagram_ord += 1;
        }
    }
}

/// Lukis satu tabel yang melebihi lebar kolom dalam ScrollArea horizontal.
#[allow(clippy::too_many_arguments)]
fn paint_markmaid_table(
    ui: &mut egui::Ui,
    view: &MdView,
    tz: &markmaid::TableZone,
    origin: Pos2,
    vis: &egui::Visuals,
    blank: &Scene,
    open_req: &mut Option<usize>,
    diagram_ord: &mut usize,
) {
    let table_min = origin + Vec2::new(tz.x as f32, tz.y as f32);
    let table_rect = Rect::from_min_size(table_min, Vec2::new(tz.w as f32, tz.h as f32));
    ui.allocate_new_ui(
        egui::UiBuilder::new().max_rect(table_rect),
        |ui| {
            egui::Frame::none().show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt(("mmd-table-scroll", tz.items.start))
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.set_min_width(tz.natural_w as f32);
                        ui.set_min_height(tz.h as f32);
                        let table_origin = ui.min_rect().min;
                        let doc_origin = table_origin - Vec2::new(tz.x as f32, tz.y as f32);
                        for item in &view.scene.items[tz.items.clone()] {
                            paint_markmaid_item(
                                ui,
                                view,
                                item,
                                doc_origin,
                                vis,
                                blank,
                                open_req,
                                diagram_ord,
                            );
                        }
                        // Tautan di dalam sel tabel — hit-test dengan scroll.
                        for (i, lz) in view.scene.links.iter().enumerate() {
                            if link_in_zone(lz, tz) {
                                let rect = Rect::from_min_size(
                                    doc_origin + Vec2::new(lz.x as f32, lz.y as f32),
                                    Vec2::new(lz.w as f32, lz.h as f32),
                                );
                                let resp = ui.interact(
                                    rect,
                                    egui::Id::new(("mmd-link-tbl", i)),
                                    Sense::click(),
                                );
                                if resp.hovered() {
                                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                                }
                                if resp.clicked() {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(lz.url.clone()));
                                }
                            }
                        }
                    });
            });
        },
    );
}

/// Lukis satu code block (``` ... ```) plus tombol Copy di pojok kanan atas.
fn paint_markmaid_code_block(
    ui: &mut egui::Ui,
    cz: &markmaid::CodeBlockZone,
    origin: Pos2,
) {
    let block_min = origin + Vec2::new(cz.x as f32, cz.y as f32);
    let btn_size = 24.0_f32;
    let pad = 4.0_f32;
    let btn_min = block_min
        + Vec2::new(
            (cz.w as f32 - btn_size - pad).max(pad),
            pad,
        );
    let btn_rect = Rect::from_min_size(btn_min, Vec2::new(btn_size, btn_size));
    ui.allocate_new_ui(
        egui::UiBuilder::new().max_rect(btn_rect),
        |ui| {
            ui.set_min_size(Vec2::new(btn_size, btn_size));
            if ui.button("📋").on_hover_text("Salin ke clipboard").clicked() {
                ui.ctx().copy_text(cz.source.clone());
            }
        },
    );
}

/// Lukis satu `DocScene` markmaid ke `ui`, berpangkal di `origin`
/// (skala 1:1 — layout sudah dihitung pada lebar panel). Diagram bisa
/// diklik untuk dibuka sebagai tab (lewat `open_req`); tautan yang
/// diklik dibuka di browser. Tabel yang melebihi lebar kolom di-render
/// dalam ScrollArea horizontal tersendiri.
pub(crate) fn paint_docscene(
    ui: &mut egui::Ui,
    view: &MdView,
    origin: Pos2,
    open_req: &mut Option<usize>,
) {
    // Snapshot tema (owned) supaya pemetaan warna tak menahan pinjaman
    // `ui` saat nanti memanggil `ui.interact`/`ui.output_mut`.
    let vis = ui.visuals().clone();
    // Scene kosong untuk pie/sequence: paint_diagram butuh `&Scene`,
    // tapi node/edge-nya kosong — geometrinya ada di pie/seq sendiri.
    let blank = blank_scene(0.0, 0.0);
    let mut diagram_ord = 0usize;
    let mut table_idx = 0usize;
    let mut code_block_idx = 0usize;
    let mut i = 0;
    while i < view.scene.items.len() {
        if let Some(tz) = view.scene.tables.get(table_idx).filter(|tz| tz.items.start == i) {
            paint_markmaid_table(ui, view, tz, origin, &vis, &blank, open_req, &mut diagram_ord);
            i = tz.items.end;
            table_idx += 1;
            continue;
        }
        if let Some(cz) = view.scene.code_blocks.get(code_block_idx).filter(|cz| cz.items.start == i) {
            for item in &view.scene.items[cz.items.clone()] {
                paint_markmaid_item(
                    ui,
                    view,
                    item,
                    origin,
                    &vis,
                    &blank,
                    open_req,
                    &mut diagram_ord,
                );
            }
            paint_markmaid_code_block(ui, cz, origin);
            i = cz.items.end;
            code_block_idx += 1;
            continue;
        }
        paint_markmaid_item(
            ui,
            view,
            &view.scene.items[i],
            origin,
            &vis,
            &blank,
            open_req,
            &mut diagram_ord,
        );
        i += 1;
    }

    // Tautan di luar tabel: zona hit-test dari markmaid → klik membuka URL di browser.
    for (i, lz) in view.scene.links.iter().enumerate() {
        if view.scene.tables.iter().any(|tz| link_in_zone(lz, tz)) {
            continue;
        }
        let rect = Rect::from_min_size(
            origin + Vec2::new(lz.x as f32, lz.y as f32),
            Vec2::new(lz.w as f32, lz.h as f32),
        );
        let resp = ui.interact(rect, egui::Id::new(("mmd-link", i)), Sense::click());
        if resp.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(lz.url.clone()));
        }
    }
}
