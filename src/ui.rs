//! Lapisan egui: loop `update`, menu, tab bar, explorer, toolbar,
//! status bar, editor teks, dan kanvas diagram interaktif.

use crate::app::App;
use crate::md::paint_docscene;
use crate::model::{Model, Pending, View};
use crate::paint::{paint_diagram, DiagramRefs};
use crate::paint_static::{draw_journey, draw_mindmap};
use crate::theme::LABEL_BORDER;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::path::{Path, PathBuf};

/// Chevron ▸/▾ ala VSCode, digambar dua garis — glyph panah font
/// tidak seragam antar-platform.
fn chevron(p: &egui::Painter, c: Pos2, open: bool, stroke: Stroke) {
    if open {
        p.line_segment([Pos2::new(c.x - 3.2, c.y - 1.6), Pos2::new(c.x, c.y + 1.9)], stroke);
        p.line_segment([Pos2::new(c.x, c.y + 1.9), Pos2::new(c.x + 3.2, c.y - 1.6)], stroke);
    } else {
        p.line_segment([Pos2::new(c.x - 1.6, c.y - 3.2), Pos2::new(c.x + 1.9, c.y)], stroke);
        p.line_segment([Pos2::new(c.x + 1.9, c.y), Pos2::new(c.x - 1.6, c.y + 3.2)], stroke);
    }
}

/// Galley satu baris yang dipotong dengan '…' bila melebihi `max_w` —
/// nama file panjang tidak boleh wrap seperti label egui default.
fn ellipsized(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_w,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts(|f| f.layout_job(job))
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
        let dirs: Vec<String> = self.open_dirs.iter().map(|p| p.display().to_string()).collect();
        storage.set_string("open_dirs", dirs.join("\n"));
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
                    // Section header ala VSCode: chevron + nama folder
                    // tebal (klik melipat seluruh pohon); menu aksi
                    // tetap di kanan dan nama terpotong rapi.
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
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
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 20.0),
                                Sense::click(),
                            );
                            let name = ws
                                .file_name()
                                .map(|n| n.to_string_lossy().to_uppercase())
                                .unwrap_or_else(|| "FOLDER".into());
                            resp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                            });
                            if ui.is_rect_visible(rect) {
                                let v = ui.visuals().clone();
                                if resp.has_focus() {
                                    ui.painter().rect_stroke(
                                        rect.shrink(0.5),
                                        0.0,
                                        v.selection.stroke,
                                    );
                                }
                                chevron(
                                    ui.painter(),
                                    Pos2::new(rect.left() + 7.0, rect.center().y),
                                    self.tree_root_open,
                                    Stroke::new(1.5_f32, v.weak_text_color()),
                                );
                                let galley = ellipsized(
                                    ui,
                                    &name,
                                    FontId::proportional(11.5),
                                    v.strong_text_color(),
                                    (rect.width() - 22.0).max(8.0),
                                );
                                let pos = Pos2::new(
                                    rect.left() + 17.0,
                                    rect.center().y - galley.size().y * 0.5,
                                );
                                // Faux bold: egui tak membawa font bold.
                                ui.painter().galley(
                                    pos,
                                    std::sync::Arc::clone(&galley),
                                    v.strong_text_color(),
                                );
                                ui.painter().galley(
                                    pos + Vec2::new(0.4, 0.0),
                                    galley,
                                    v.strong_text_color(),
                                );
                            }
                            if resp.clicked() {
                                self.tree_root_open = !self.tree_root_open;
                            }
                        });
                    });
                    if self.tree_root_open {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            self.draw_tree(ui, &ws, 0);
                            ui.add_space(4.0);
                        });
                    }
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
                            format!("markdown · {} diagram", m.block_srcs.len())
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
                                Model::Mindmap(d) => {
                                    format!("mindmap · {} node", d.nodes.len())
                                }
                                Model::Journey(d) => {
                                    let n: usize = d.sections.iter().map(|s| s.tasks.len()).sum();
                                    format!("journey · {} task", n)
                                }
                                Model::GitGraph(d) => {
                                    format!("gitGraph · {} commit", d.commits.len())
                                }
                                Model::Architecture(d) => {
                                    format!("architecture · {} service", d.services.len())
                                }
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

    /// Pohon file rekursif ala explorer VSCode: baris digambar manual
    /// (bukan CollapsingHeader) demi highlight hover/aktif selebar
    /// panel, chevron + ikon vektor, indent guide, dan nama satu baris
    /// ter-ellipsis. Klik folder melipat via `open_dirs`; klik file
    /// membuka tab (file aktif di-highlight).
    fn draw_tree(&mut self, ui: &mut egui::Ui, dir: &Path, depth: usize) {
        const ROW_H: f32 = 22.0;
        const INDENT: f32 = 10.0;
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
            // Alokasi hanya menentukan tinggi baris; interaksi dipasang
            // pada rect selebar panel supaya area klik = area highlight
            // (allocate + Sense::click hanya seluas rect ber-margin).
            let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), ROW_H));
            let row = Rect::from_x_y_ranges(ui.clip_rect().x_range(), rect.y_range());
            let mut resp = ui.interact(row, egui::Id::new(("tree-row", &t.path)), Sense::click());
            let open = t.is_dir && self.open_dirs.contains(&t.path);
            let selected = !t.is_dir && self.path.as_deref() == Some(t.path.as_path());
            // Tanpa info widget, baris manual = node anonim bagi screen
            // reader (AccessKit sengaja dipertahankan — lihat Cargo.toml).
            resp.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    selected || open,
                    &t.name,
                )
            });
            if ui.is_rect_visible(rect) {
                let v = ui.visuals().clone();
                let p = ui.painter();
                if selected {
                    p.rect_filled(row, 0.0, v.selection.bg_fill.gamma_multiply(0.55));
                } else if resp.hovered() {
                    let c = if v.dark_mode {
                        Color32::from_white_alpha(10)
                    } else {
                        Color32::from_black_alpha(10)
                    };
                    p.rect_filled(row, 0.0, c);
                }
                // Fokus keyboard (Tab lalu Enter/Space) harus terlihat.
                if resp.has_focus() {
                    p.rect_stroke(row.shrink(0.5), 0.0, v.selection.stroke);
                }
                let x0 = rect.left() + 4.0 + depth as f32 * INDENT;
                let cy = rect.center().y;
                // Indent guide tipis di kolom chevron tiap leluhur.
                let guide = if v.dark_mode {
                    Color32::from_white_alpha(14)
                } else {
                    Color32::from_black_alpha(20)
                };
                for l in 0..depth {
                    let gx = rect.left() + 4.0 + l as f32 * INDENT + 5.0;
                    p.line_segment(
                        [Pos2::new(gx, rect.top()), Pos2::new(gx, rect.bottom())],
                        Stroke::new(1.0_f32, guide),
                    );
                }
                let muted = v.weak_text_color();
                if t.is_dir {
                    chevron(p, Pos2::new(x0 + 5.0, cy), open, Stroke::new(1.5_f32, muted));
                    // Ikon folder: tab kecil + badan.
                    let fx = x0 + 13.0;
                    let fill = muted.gamma_multiply(0.8);
                    p.rect_filled(
                        Rect::from_min_size(Pos2::new(fx, cy - 5.5), Vec2::new(5.0, 2.5)),
                        1.0,
                        fill,
                    );
                    p.rect_filled(
                        Rect::from_min_size(Pos2::new(fx, cy - 3.8), Vec2::new(11.0, 8.8)),
                        1.5,
                        fill,
                    );
                } else {
                    // Ikon file: halaman bergaris, warna per jenis file.
                    let ext = t
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase());
                    // Varian lebih gelap di tema terang agar kontras.
                    let tint = match (ext.as_deref(), v.dark_mode) {
                        (Some("mmd"), true) => Color32::from_rgb(0xc9, 0x71, 0xb8),
                        (Some("mmd"), false) => Color32::from_rgb(0x9c, 0x3f, 0x8c),
                        (Some("md" | "markdown"), true) => Color32::from_rgb(0x5f, 0xa8, 0xd3),
                        (Some("md" | "markdown"), false) => Color32::from_rgb(0x2c, 0x6f, 0x94),
                        _ => muted,
                    };
                    let fx = x0 + 14.0;
                    p.rect_stroke(
                        Rect::from_min_size(Pos2::new(fx, cy - 5.5), Vec2::new(9.0, 11.0)),
                        1.0,
                        Stroke::new(1.2_f32, tint),
                    );
                    let ink = Stroke::new(1.0_f32, tint.gamma_multiply(0.7));
                    for (dy, len) in [(-1.5, 5.0), (1.0, 5.0), (3.5, 3.0)] {
                        p.line_segment(
                            [Pos2::new(fx + 2.0, cy + dy), Pos2::new(fx + 2.0 + len, cy + dy)],
                            ink,
                        );
                    }
                }
                // Nama: satu baris, sisa lebar, dipotong dengan ….
                let tx = x0 + 28.0;
                let color = if selected { v.strong_text_color() } else { v.text_color() };
                let galley = ellipsized(
                    ui,
                    &t.name,
                    FontId::proportional(13.0),
                    color,
                    (rect.right() - 6.0 - tx).max(8.0),
                );
                // Nama yang terpotong tetap terbaca utuh via tooltip.
                if galley.elided {
                    resp = resp.on_hover_text(&t.name);
                }
                let ty = cy - galley.size().y * 0.5;
                ui.painter().galley(Pos2::new(tx, ty), galley, color);
            }
            if resp.clicked() {
                if t.is_dir {
                    if open {
                        self.open_dirs.remove(&t.path);
                    } else {
                        self.open_dirs.insert(t.path.clone());
                    }
                } else if !selected {
                    self.open_path(t.path.clone());
                }
            }
            if t.is_dir && open {
                self.draw_tree(ui, &t.path, depth + 1);
            }
        }
    }

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
        // Ambil-sementara supaya interaksi diagram/tautan bisa
        // meminjam &mut self (mdoc di-relayout di sini).
        let Some(mut mdoc) = self.mdoc.take() else { return };
        let mut open_req: Option<usize> = None;
        // Batasi lebar prosa supaya baris tidak terlalu panjang;
        // markmaid sudah memberi margin dalam sendiri.
        const MAX_W: f32 = 860.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let width = ui.available_width().clamp(1.0, MAX_W);
                // Layout hanya dihitung ulang saat lebar berubah —
                // mengetik/scroll tidak membayar layout tiap frame.
                if (width - mdoc.laid_width).abs() > 0.5 {
                    ui.fonts(|fonts| mdoc.relayout(width, fonts));
                }
                let size = Vec2::new(mdoc.scene.width as f32, mdoc.scene.height as f32);
                let (resp, _painter) = ui.allocate_painter(size, egui::Sense::hover());
                paint_docscene(ui, &mdoc, resp.rect.min, &mut open_req);
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
        if let Some(ms) = &self.mind {
            // Mindmap: kotak node di-hit-test langsung (bukan Scene node);
            // menggeser mengubah `pos`, lalu route() menyambung ulang.
            for i in 0..ms.nodes.len() {
                let b = &ms.nodes[i];
                let rect = Rect::from_min_max(ts(b.x, b.y), ts(b.x + b.w, b.y + b.h));
                let resp = ui.interact(rect, egui::Id::new(("mind-node", i)), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                    hovered_node = Some(i);
                }
                if resp.dragged() {
                    let d = resp.drag_delta() / zoom;
                    self.pos[i].0 += d.x as f64;
                    self.pos[i].1 += d.y as f64;
                    moved = true;
                }
            }
        } else {
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
        }
        if moved {
            self.dragged = true;
            self.reroute();
        }

        // 2) Lukis diagram lewat fungsi bersama (juga dipakai
        //    pratinjau inline dokumen Markdown). Mindmap punya painter
        //    sendiri (bukan Scene node/edge) — di atas kertas putih
        //    yang sama seperti pie/sequence.
        let painter = ui.painter();
        if let Some(ms) = &self.mind {
            let paper = Rect::from_min_max(ts(0.0, 0.0), ts(ms.width, ms.height))
                .expand(12.0 * zoom);
            painter.rect(paper, 8.0 * zoom, Color32::WHITE, Stroke::new(1.0_f32, LABEL_BORDER));
            draw_mindmap(painter, ms, &ts, zoom);
        } else if let Some(jsc) = &self.journey {
            let paper = Rect::from_min_max(ts(0.0, 0.0), ts(jsc.width, jsc.height));
            painter.rect(paper, 8.0 * zoom, Color32::WHITE, Stroke::new(1.0_f32, LABEL_BORDER));
            draw_journey(painter, jsc, &ts, zoom);
        } else {
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
