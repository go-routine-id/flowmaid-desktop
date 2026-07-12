# flowmaid-desktop

Interactive desktop diagram editor built on the [flowmaid](https://crates.io/crates/flowmaid) engine (eframe/egui, pure Rust).

## Features

- **Live Mermaid editing** — type `flowchart` or `erDiagram` text on the left, see the diagram instantly. A *last good render* pattern means half-typed text never flashes an error frame, and dragged positions are preserved by id while typing.
- **Drag nodes with the mouse** — edges re-route in realtime via `flowmaid::scene::route`.
- **ER diagrams** — entities render as attribute tables with crow's foot cardinality notation (`flowmaid::er`), draggable like everything else.
- **Zoom & pan** — pinch / ctrl+scroll / ± buttons to zoom (anchored at the cursor), scroll or drag empty canvas to pan; click the percentage button to reset.
- **Real file workflow** — File menu with New / Open… / recent files / Save / Save As… (⌘N ⌘O ⌘S ⇧⌘S), a dirty indicator in the window title, and a save-first confirmation before anything discards unsaved changes. Files also open via drag & drop or a command-line path.
- **Auto re-layout** restores the engine layout; **Export SVG…** saves the current arrangement, drags included.

## Running

```bash
cargo run --release              # needs Rust >= 1.85 (eframe dependency)
cargo run --release -- file.mmd  # open a file directly
```

The engine itself stays pure-std and Rust 1.75-compatible; the newer toolchain requirement comes only from the GUI dependencies.

Note: the UI labels are currently in Indonesian.

## License

GPL-3.0-or-later — same as the flowmaid engine it links against. Full text in `LICENSE`.
