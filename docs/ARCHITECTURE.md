# Architecture

This is a map of texcopy for anyone who wants to change it. For what it does, see the [README](../README.md). For the model details see [HOW_IT_WORKS.md](HOW_IT_WORKS.md), for what it writes see [OUTPUT.md](OUTPUT.md), and for how it's measured see [BENCHMARKS.md](BENCHMARKS.md).

## The shape of it

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

`texcopy` with no arguments opens the window. `texcopy <image>` and the other flags are the command line. They both run the same code: `src/lib.rs` has `run()`, and `src/main.rs` and `src/bin/texcopy-gui.rs` are two thin launchers around it. The second one only exists so Windows can have a build with no console window (`windows_subsystem = "windows"`). One `.exe` can't be both a console program and a windowless app.

## Where things are

| Path | What it does |
|---|---|
| `src/lib.rs` | the command line (`clap`), input routing, printing, `run()` |
| `src/engine.rs` | `Options` (what to do), `Warm` (what stays loaded between reads), `read()` (which engine, and the fallbacks) |
| `src/gui.rs` | the window: the UI thread, one worker thread, a short capture thread per click, settings, the first-run download card |
| `src/capture.rs` | picking a region: the OS's own tool first, then `overlay.rs`. Also `--screen` and `--region` |
| `src/overlay.rs` | texcopy's own selector, run as a separate process (`--select-overlay`) |
| `src/clipboard.rs` | read an image, write text. On Linux a small detached process keeps serving the text |
| `src/model.rs`, `src/spec.rs` | the small ONNX formula model and its description (`ModelSpec`, optional `spec.json`) |
| `src/tidy.rs`, `src/prose.rs` | clean up the small model's tokens, and prose mode for sentences |
| `src/vlm.rs` | the vision engine: the setup plan, the `llama-server` process, the request, deciding formula or text |
| `src/helper.rs`, `src/fetch.rs`, `src/ui.rs` | the pinned `llama.cpp` release, checksum-verified resumable downloads, progress events |
| `src/convert.rs` | the vision model's Markdown into LaTeX: headings, lists, display equations, paragraphs, escapes |
| `src/table.rs` | HTML (or plain-line) tables into `tabular` |
| `tools/` | the benchmark (`bench.py`), the real-textbook case maker, the model comparison, the license-notice generator |
| `tools/gen/` | the Rust program that makes generated benchmark cases (random formulas, documents, web-style pictures, real arXiv LaTeX). Its own crate, not part of the shipped program |
| `packaging/` | the scripts that build the macOS `.app`, the Windows zip and the Linux tarball |
| `.github/workflows/` | `ci.yml` (format, lint, build, package, minimum Rust), `tests.yml`, `release.yml` |

## How it picks an engine (`engine::read`)

1. `--engine small`: the small model only.
2. `--engine vlm`: the vision model only. It gets set up first if it needs to be.
3. `auto` (the default): the vision model first, *if it's already set up*. If it read real text, that's the answer. If it only read a formula, the small model reads it as well, because it's the better formula reader. The vision formula only gets used if the small model gets stuck repeating itself. If the vision engine is missing or fails, the small model answers and the command line prints a tip.

`vlm::interpret` decides text or formula from what the model wrote. A real word next to some math, or three or more real words, is text. Two or more lines of plain text are never a formula. Everything else is a bare formula.

## Threads and what stays loaded (`gui.rs`)

- The **UI thread** draws and never blocks.
- The **worker thread** owns `Warm` (the running `llama-server` and the loaded small model) and takes jobs over a channel: read an image, install the vision engine, warm up. After 150 s with no job it unloads everything (about 1.4 GB).
- A **capture thread** per click hides the window, waits for it to disappear, and blocks in the selector.
- **Warm-up** starts the vision server when the window opens and again when you hit Capture, so it's already loading while you drag the box. It never downloads anything and never asks.
- **Shutdown has to join the worker**, because the worker owns the helper process. If texcopy crashes and leaves a helper behind, the next texcopy run stops it. Each run writes a note with its helper's pid, and a later run kills helpers whose owner is gone (only if the process really is `llama-server`).

## Downloads and trust

Everything fetched at runtime is pinned in the source. The `llama.cpp` release and the model files have sizes and SHA-256 checksums (`src/helper.rs`, `src/vlm.rs`). Downloads go to a `.part` file, resume where they stopped, get verified, and a mismatch deletes the file. Setup is a plan (`vlm::plan`). The command line prints it and asks, and the window shows it as a card with a progress bar. Both run the same `Plan::install` and report `ui::Event`s.

## Why it's built this way

| Decision | Why |
|---|---|
| Two engines, vision model optional | The 118 MB formula model runs anywhere and is great on single formulas. The 1 GB vision model reads text, long equations and tables. You pick whether to download it. |
| Vision model through `llama-server`, not in-process | A Rust-native runtime (Candle) measured 72-78 s on Metal and 4.7-6.8 s on CPU, against about 0.3 s for `llama.cpp`. The helper also keeps the program small. |
| `llama-server --cache-ram 0` | The default prompt cache grows the process from 1.4 GB to over 3 GB after a few dozen images. |
| Use the OS's own region selector when there is one | A custom overlay fought tiling window managers and menu bars. The system tools don't, and they work on Wayland. |
| The built-in overlay is its own process | It has to be the first window its process makes (a second window can get moved by the window manager), and it has to take the screenshot *before* it makes a window so it isn't in its own picture. |
| The window has no minimize or zoom buttons | Tiling window managers (AeroSpace) treat a window like that as a dialog and leave it floating. |
| The output is "a correct reconstruction" | A picture can't say which LaTeX made it. Bold, italic and table rules aren't recoverable, so they're out of scope. The benchmark grades by re-rendering the output. |
| Stretch, not pad, when resizing for the small model | It matches the model's training preprocessing and does better on wide formulas. |
| Confidence warning at 0.5 | No false alarms on exact results in the benchmark. 0.7 flagged 12 of 45. |
| `ModelSpec` and `spec.json` | You can swap in another encoder-decoder model, or one you trained, without touching the code ([CUSTOM_MODELS.md](CUSTOM_MODELS.md)). |
| Supported: macOS Apple Silicon, Windows x64, Linux x64 | There's no prebuilt ONNX Runtime for Intel Macs in the `ort` version used. |
| Fat LTO, `panic = "abort"`, stripped, no `opt-level = "s"` | It saved 1-2 MB, but screen captures got 13-57% slower. About 78% of the binary is ONNX Runtime anyway, so compiler flags don't reach far. |

## Testing

- `cargo test` runs the unit tests that sit next to the code. They include a local HTTP server for the download logic, fake programs for the Linux selector chain, and real model outputs for the text conversion.
- `tools/bench.py` renders known LaTeX, runs texcopy, re-renders the answer and compares (see [BENCHMARKS.md](BENCHMARKS.md)). Compare against the previous build, not just the totals.
- `tools/gen` makes benchmark cases from a seed (random formulas and documents, KaTeX pages, real CC-BY arXiv LaTeX), so the benchmark isn't only things I thought to write by hand. `texcopy --batch list.txt` reads many pictures with one set of loaded models, which keeps a 200-case run to a few minutes.
- The window can test itself without a mouse. `texcopy --gui-selftest picture.png` "captures" a picture, reads it twice, and checks that the window kept running while hidden and that the second read reused the warm engine. `TEXCOPY_GUI_SCREENSHOT=out.png` saves a picture of the window, and `TEXCOPY_OVERLAY_AUTO=x0,y0,x1,y1` scripts a drag in the overlay.

## Releasing

Set `version` in `Cargo.toml`, commit, and push a tag like `vX.Y.Z`. `release.yml` builds the three platforms, runs the tests, packages each one (`packaging/`), and publishes a GitHub release with SHA-256 checksums. The macOS app is ad-hoc signed and not notarised, and the Windows build is unsigned.

## What's still rough

The honest list is in [BENCHMARKS.md](BENCHMARKS.md#limitations). The big one: I've only run it by hand on macOS. The Windows and Linux builds, the built-in overlay on those systems, and picking a region on a second monitor have been built and tested by CI but not tried on real machines.
