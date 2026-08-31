use crate::app::{App, MAX_ZOOM, MIN_ZOOM};
use crate::md::build_mdview;
use crate::model::{Model, Pending, TreeEntry, CONTOH};
use crate::theme::hex;
use eframe::egui::{Color32, Vec2};
use std::path::Path;
use std::rc::Rc;

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
    // Positions preserved by key across an edit — geser satu
    // entitas seperti handler drag (yang juga menyetel `dragged`;
    // tanpa flag itu reparse mengikuti auto-layout sepenuhnya).
    let before = a.pos[0];
    a.pos[0] = (before.0 + 300.0, before.1);
    a.dragged = true;
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
    // Ekstraksi (scanner teks markmaid): dua blok mermaid, fence
    // js dilewati.
    let blocks = markmaid::blocks::mermaid_blocks(md);
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].0.starts_with("flowchart TD"));
    assert!(blocks[1].0.starts_with("pie"));

    // Splice mengganti isi blok #2 tanpa menyentuh sekitarnya.
    let out = markmaid::blocks::splice(md, 1, "pie\n\"y\" : 9").unwrap();
    assert!(out.contains("~~~mermaid\npie\n\"y\" : 9\n~~~"));
    assert!(out.contains("console.log(1)") && out.contains("penutup"));
    assert!(out.contains("A-->B"), "blok #1 tak tersentuh");

    // Parse+layout markmaid: dua diagram tertanam, heading, dan
    // fence non-mermaid jadi blok kode biasa.
    let mut rendered = build_mdview(md);
    rendered.relayout_estimated(600.0);
    let diagrams = rendered
        .scene
        .items
        .iter()
        .filter(|i| matches!(i, markmaid::Item::Diagram(_)))
        .count();
    assert_eq!(diagrams, 2, "dua diagram inline ter-layout");
    assert!(rendered
        .doc
        .blocks
        .iter()
        .any(|b| matches!(b, markmaid::Block::Heading { level: 1, .. })));
    assert!(
        rendered
            .doc
            .blocks
            .iter()
            .any(|b| matches!(b, markmaid::Block::Code { lang, .. } if lang == "js")),
        "fence js jadi blok kode biasa"
    );

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
    assert_eq!(m.block_srcs.len(), 2, "dua sumber blok mermaid");
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
fn gfm_tables_tasks_strike_and_links_render() {
    let md = "# T\n\n~~coret~~ dan [tautan](https://x.dev) di sini\n\n\
              | Kolom A | Kolom B |\n|---|---|\n| a1 | b1 |\n| a2 | b2 |\n\n\
              - [x] beres\n- [ ] belum\n- biasa\n";
    // Pipeline markmaid penuh: parse → layout → DocScene.
    let mut v = build_mdview(md);
    v.relayout_estimated(600.0);
    let items = &v.scene.items;
    // Tautan → zona klik dengan URL-nya (bisa dibuka browser).
    assert!(
        v.scene.links.iter().any(|l| l.url == "https://x.dev"),
        "tautan jadi LinkZone"
    );
    // Strikethrough → TextRun.strike.
    assert!(
        items.iter().any(|it| matches!(it, markmaid::Item::Text(t)
            if t.strike && t.text.contains("coret"))),
        "strikethrough ter-render"
    );
    // Tabel GFM menggambar garis grid (LineItem).
    assert!(
        items.iter().any(|it| matches!(it, markmaid::Item::Line(_))),
        "tabel menggambar garis grid"
    );
    // Header tabel ada sebagai teks.
    assert!(
        items.iter().any(|it| matches!(it, markmaid::Item::Text(t)
            if t.text.contains("Kolom A"))),
        "header tabel ter-render"
    );
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
