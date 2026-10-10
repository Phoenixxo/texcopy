# Architecture

A map of texcopy for people who want to change it. For *what it does* see the [README](../README.md); for the model internals see [HOW_IT_WORKS.md](HOW_IT_WORKS.md); for what it writes see [OUTPUT.md](OUTPUT.md); for how it is measured see [BENCHMARKS.md](BENCHMARKS.md).

## The shape of the program

```
                       ┌──────────── src/gui.rs ────────────┐
 Capture click ──▶ capture thread ──▶ image ──▶ worker thread (owns the loaded engines)
 (src/capture.rs)   hides the window,           │
                    runs the selector           ▼
                                      src/engine.rs  ── picks ──┬─▶ small model  (src/model.rs, ONNX in-process)
                                                                └─▶ vision model (src/vlm.rs ─▶ llama-server)
                                                                              │
                              LaTeX ◀── src/convert.rs, src/table.rs ◀── the model's Markdown / HTML
```

`texcopy` (no arguments) opens the window; `texcopy <image>` and the other flags are the command line. Both call the same code: `src/lib.rs` holds `run()`, and `src/main.rs` and `src/bin/texcopy-gui.rs` are two thin launchers around it. The second exists only so that on Windows it can be built without a console window (`windows_subsystem = "windows"`); a single `.exe` cannot be both a console program and a windowless app.

## Source map

| Path | Responsibility |
|---|---|
| `src/lib.rs` | the command-line definition (`clap`), input routing, output printing, `run()` |
| `src/engine.rs` | `Options` (what to do), `Warm` (what stays loaded between reads), `read()` (which engine, and the fallbacks between them) |
| `src/gui.rs` | the window: UI thread, one worker thread, one short capture thread per click; settings; the first-run download card |
| `src/capture.rs` | selecting a region: the OS's own tool first, then `overlay.rs`; also `--screen` and `--region` |
| `src/overlay.rs` | texcopy's own selector, run as a separate process (`--select-overlay`) |
| `src/clipboard.rs` | read an image / write text; on Linux a small detached process keeps serving the text |
| `src/model.rs`, `src/spec.rs` | the small ONNX formula model and its description (`ModelSpec`, optional `spec.json`) |
| `src/tidy.rs`, `src/prose.rs` | clean up the small model's tokens; prose mode for sentences |
| `src/vlm.rs` | the vision engine: setup plan, `llama-server` process, request, deciding formula vs text |
| `src/helper.rs`, `src/fetch.rs`, `src/ui.rs` | pinned `llama.cpp` release, checksum-verified resumable downloads, progress events |
| `src/convert.rs` | the vision model's Markdown to LaTeX: headings, lists, display equations, paragraphs, escapes |
| `src/table.rs` | HTML (or plain-line) tables to `tabular` |
| `tools/` | the benchmark (`bench.py`), real-textbook case generator, model comparison, license-notice generator |
| `packaging/` | scripts that build the macOS `.app`, the Windows zip and the Linux tarball |
| `.github/workflows/` | `ci.yml` (format, lint, build, package, minimum Rust), `tests.yml`, `release.yml` |

## Choosing an engine (`engine::read`)

1. `--engine small`: the small model only.
2. `--engine vlm`: the vision model only; it is set up first if needed.
3. `auto` (default): the vision model first, *if it is already set up*. If it read real text, that is the answer. If it read only a formula, the small model reads it too, because it is the better formula reader, and the vision formula is used only if the small model gets stuck repeating itself. If the vision engine is missing or fails, the small model answers and the command line prints a tip.

`vlm::interpret` decides text versus formula from the model's output: a real word next to math, or three or more real words, is text; two or more lines of plain text are never a formula; everything else is a bare formula.

## Threads and what stays loaded (`gui.rs`)

- The **UI thread** draws and never blocks.
- The **worker thread** owns `Warm` (the running `llama-server` and the loaded small model) and receives jobs over a channel: read an image, install the vision engine, warm up. After 150 s without a job it unloads everything (about 1.4 GB).
- A **capture thread** per click hides the window, waits for it to disappear, and blocks in the selector.
- **Warm-up** starts the vision server when the window opens and again on Capture, so it is loading while the user drags out a region. It never downloads or asks.
- **Shutdown must join the worker**, because the worker owns the helper process. A helper left behind after a crash is stopped by the next texcopy run: each run writes a note with its helper's pid, and a later run kills helpers whose owner no longer exists (only if the process really is `llama-server`).

## Downloads and trust

Everything fetched at runtime is pinned in the source: the `llama.cpp` release and the model files carry sizes and SHA-256 checksums (`src/helper.rs`, `src/vlm.rs`). Downloads go to a `.part` file, resume where they stopped, are verified, and a mismatch deletes the file. Setup is described as a plan (`vlm::plan`) that the command line prints and asks about, and the window shows as a card with a progress bar; both run the same `Plan::install`, reporting `ui::Event`s.

## Design decisions and why

| Decision | Why |
|---|---|
| Two engines, vision model optional | The 118 MB formula model runs anywhere and is excellent on single formulas; the 1 GB vision model reads text, long equations and tables. Users choose the download. |
| Vision model through `llama-server`, not in-process | A Rust-native runtime (Candle) was measured at 72-78 s on Metal and 4.7-6.8 s on CPU, against about 0.3 s for `llama.cpp`. The helper also keeps the program small. |
| `llama-server --cache-ram 0` | The default prompt cache grows the process from 1.4 GB past 3 GB after a few dozen images. |
| Use the OS's own region selector where there is one | A custom overlay fought tiling window managers and menu bars. The system tools are immune, and work on Wayland. |
| The built-in overlay is its own process | The overlay must be the first window its process creates (a second window may be moved by the window manager), and must capture the screen *before* creating a window so it is not in its own picture. |
| The window has no minimize or zoom buttons | Tiling window managers (AeroSpace) treat such a window as a dialog and leave it floating. |
| Output is "a correct reconstruction" | A picture cannot say which LaTeX produced it. Bold, italic and table rules are not recoverable, so they are out of scope; the benchmark grades by re-rendering the output. |
| Stretch, not pad, when resizing for the small model | It matches the model's training preprocessing and wins on wide formulas. |
| Confidence warning at 0.5 | Zero false alarms on exact results in the benchmark; 0.7 flagged 12 of 45. |
| `ModelSpec` and `spec.json` | Swap in another encoder-decoder model, or one you train, without code changes ([CUSTOM_MODELS.md](CUSTOM_MODELS.md)). |
| Supported: macOS Apple Silicon, Windows x64, Linux x64 | No prebuilt ONNX Runtime is available for Intel Macs in the `ort` version used. |
| Fat LTO, `panic = "abort"`, stripped; not `opt-level = "s"` | The size saving was 1-2 MB, but screen captures became 13-57% slower. About 78% of the binary is ONNX Runtime, so compiler flags have limited reach. |

## Testing

- `cargo test`: unit tests beside the code, including a local HTTP server for the download logic, fake programs for the Linux selector chain, and the model's real outputs for text conversion.
- `tools/bench.py`: renders known LaTeX, runs texcopy, re-renders the answer and compares (see [BENCHMARKS.md](BENCHMARKS.md)). Compare against the previous build, not just the totals.
- The window can test itself without a mouse: `texcopy --gui-selftest picture.png` "captures" a picture, reads it twice, and checks that the window kept running while hidden and that the second read reused the warm engine. `TEXCOPY_GUI_SCREENSHOT=out.png` saves a picture of the window; `TEXCOPY_OVERLAY_AUTO=x0,y0,x1,y1` scripts a drag in the overlay.

## Releasing

Set `version` in `Cargo.toml`, commit, and push a tag `vX.Y.Z`. `release.yml` builds the three platforms, runs the tests, packages each (`packaging/`), generates the license notices that ship in every download (`tools/third_party_licenses.py`), and publishes a GitHub release with SHA-256 checksums. The macOS app is ad-hoc signed and not notarised; the Windows build is unsigned.

## Known gaps

The honest list is in [BENCHMARKS.md](BENCHMARKS.md#limitations). The largest: only macOS has been run by hand. The Windows and Linux builds, the built-in overlay on those systems, and multi-monitor selection have been built and tested by CI but not exercised on real machines.
