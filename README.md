# flowmaid-desktop

[![CI](https://github.com/go-routine-id/flowmaid-desktop/actions/workflows/ci.yml/badge.svg)](https://github.com/go-routine-id/flowmaid-desktop/actions/workflows/ci.yml)
Interactive desktop diagram editor built on the [flowmaid](https://crates.io/crates/flowmaid) engine (eframe/egui, pure Rust).

## Features

- **VSCode-style layout** — a folder explorer on the left (open a folder, click `.mmd` files to open them), and a **Preview | Split | Code** main area. Split (the default) shows the diagram and the text side by side; Preview and Code give each the full width.
- **Live Mermaid editing** — type `flowchart` or `erDiagram`, watch the diagram update as you type. A *last good render* pattern means half-typed text never flashes an error frame, and dragged positions are preserved by id while typing.
- **Drag nodes with the mouse** — edges re-route in realtime via `flowmaid::scene::route`.
- **ER diagrams** — entities render as attribute tables with crow's foot cardinality notation (`flowmaid::er`), draggable like everything else.
- **Zoom & pan** — pinch / ctrl+scroll / ± buttons to zoom (anchored at the cursor), scroll or drag empty canvas to pan; click the percentage button to reset.
- **Real file workflow** — File menu with New / Open… / recent files / Save / Save As… (⌘N ⌘O ⌘S ⇧⌘S), a dirty indicator in the window title, and a save-first confirmation before anything discards unsaved changes. Files also open via drag & drop or a command-line path.
- **Auto re-layout** restores the engine layout; **Export SVG…** saves the current arrangement, drags included.

## Download

Prebuilt binaries for **macOS / Linux / Windows** are on the [Releases page](https://github.com/go-routine-id/flowmaid-desktop/releases) — no Rust toolchain needed. macOS builds are unsigned for now: first launch via right-click → Open, or `xattr -d com.apple.quarantine flowmaid-desktop`.

The version number tracks the bundled flowmaid engine (desktop v0.6.x runs engine 0.6.x).

## Running from source

```bash
cargo run --release              # needs Rust >= 1.85 (eframe dependency)
cargo run --release -- file.mmd  # open a file directly
```

The engine itself stays pure-std and Rust 1.75-compatible; the newer toolchain requirement comes only from the GUI dependencies.

Note: the UI labels are currently in Indonesian.

## License

GPL-3.0-or-later — same as the flowmaid engine it links against. Full text in `LICENSE`.
