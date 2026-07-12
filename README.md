# flowmaid-demo

Desktop demo app (eframe/egui) showcasing [flowmaid](https://crates.io/crates/flowmaid) as an interactive diagram engine:

- **Drag nodes with the mouse** — edges re-route in realtime via `flowmaid::scene::route`.
- **Zoom & pan** — pinch / ctrl+scroll / ± buttons to zoom (anchored at the cursor), scroll or drag empty canvas to pan; click the percentage button to reset.
- **Flowcharts AND ER diagrams** — open an `erDiagram` file and entities render as attribute tables with crow's foot notation (`flowmaid::er`), draggable like everything else.
- **Drop a `.mmd` file** onto the window to open it.
- Live text editor on the left with a *last good render* pattern — half-typed text never flashes an error frame, and dragged node positions are preserved by id while typing.
- **Auto re-layout** restores the engine layout; **Export SVG** saves the current arrangement, drags included.

## Running

```bash
cargo run --release              # needs Rust >= 1.85 (eframe dependency)
cargo run --release -- file.mmd  # open a file directly
```

The engine itself stays pure-std and Rust 1.75-compatible; the newer toolchain requirement comes only from this demo's GUI dependencies.

Note: the UI labels are currently in Indonesian.

## License

GPL-3.0-or-later — same as the flowmaid engine it links against. Full text in `LICENSE`.
