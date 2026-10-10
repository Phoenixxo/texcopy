# texcopy review checklist

Come back to this any time to remember what exists, why, and how to check it. Tick boxes as you go (`- [x]`). Everything here describes the state after **Phase 1** plus the **prose mode** work that followed (git history up to `9fcf054`).

## 0. The picture in one minute

texcopy turns an image of LaTeX-rendered math back into LaTeX source. Rendered output doesn't contain its source, so this is OCR done by a small neural network that runs locally.

```
image source ──► preprocess ──► ENCODER ──► 578 feature vectors ──► DECODER loop ──► tokens ──► tidy ──► clipboard
(file/clipboard/    (white bg,    (vision          (what the image       (one token at a    (text)    (readable
 screenshot/         dark→light,   transformer,      "looks like")         time, greedy;                LaTeX)
 macOS selector)     stretch to    ONNX)                                   confidence per
                     384×384)                                              token; loop guard)
```

| File | Job | Lines |
|---|---|---|
| `src/main.rs` | CLI flags, glue, JSON output, warnings | ~190 |
| `src/model.rs` | preprocess, encoder/decoder run, confidence, loop detection | ~310 |
| `src/spec.rs` | `ModelSpec`: everything model-specific, optional `spec.json` | ~100 |
| `src/fetch.rs` | find the model folder, first-run download, cache dir | ~105 |
| `src/capture.rs` | screenshots (`xcap`), `--region` crop, macOS selector | ~115 |
| `src/vlm.rs` | vision engine: setup flow, `llama-server` process, request, Markdown -> prose/formula (7 tests) | ~480 |
| `src/helper.rs` | pinned llama.cpp release + SHA-256s, install and unpack (2 tests) | ~270 |
| `src/ui.rs` | yes/no prompt, download progress bar (3 tests) | ~170 |
| `src/clipboard.rs` | read image / write text; Linux helper process | ~75 |
| `src/tidy.rs` | raw tokens → idiomatic LaTeX (10 unit tests) | ~650 |
| `src/prose.rs` | sentence with inline math → text with `$...$` (13 unit tests) | ~600 |
| `tools/bench.py` | regression benchmark (needs `pdflatex`, `pdftoppm`, `magick`) | ~300 |
| `tools/gallery.py` | visual report: inputs, outputs, confidence (writes `gallery/index.html`, gitignored) | ~200 |
| `tools/book_cases.py` | cuts real exercise crops + ground truth out of a textbook PDF into `tools/fixtures/private/` (gitignored) | ~230 |
| `tools/bakeoff.py` | scores vision-language OCR models (llama.cpp servers) on the benchmark cases | ~200 |
| `tools/fixtures/textbook-5.4.png` | your real textbook screenshot, used by the prose suite | |
| `tools/baseline.json` | saved benchmark results to compare against | |
| `.github/workflows/ci.yml` | build + lint + test on macOS ARM, Windows, Linux | |

---

## 1. Ten-minute health check

Run from the repo root. Tick each when it behaves as stated.

- [ ] `cargo test --release` → **47 passed** (prose 14, tidy 10, vlm 7, fetch 6, model 3, ui 3, helper 2, capture 2, spec 1)
- [ ] `cargo clippy --locked --all-targets -- -D warnings` → no warnings
- [ ] `cargo fmt --check` → no output
- [ ] `cargo build --release && ls -l target/release/texcopy` → about **22.9 MB** on Apple Silicon
- [ ] `python3 tools/bench.py --baseline tools/baseline.json` → totals **71 exact / 9 close / 3 differs / 1 FAIL** (84 cases), and "**0 improved, 0 regressed**" (takes about a minute; needs the LaTeX tools)
- [ ] `./target/release/texcopy some-formula.png --no-copy` prints LaTeX
- [ ] `./target/release/texcopy some-formula.png --json` shows `latex`, `kind`, `raw`, `stop`, `min_prob`, `tokens[]`
- [ ] `./target/release/texcopy tools/fixtures/textbook-5.4.png --no-copy` prints exactly `5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.`
- [ ] `python3 tools/gallery.py --open` opens a page with 17 cards

If the benchmark says "REGRESSED", something changed behaviour: find which commit (`git bisect` works well here).

---

## 2. Review by area

For each area: read the listed code, then check you can answer the questions.

### A. The recognition pipeline (`src/model.rs`)
- [ ] **`preprocess`**: flattens transparency onto white, inverts dark captures (mean brightness < 127.5), then fits to the input size (stretch or pad), then normalises with `(pixel/255 − mean)/std`.
  - Can you explain why stretch is the default? *(It matches the model's own image processor; benchmark: stress suite 9 exact vs 7 for pad.)*
- [ ] **`recognize`**: encodes once, then decodes in a loop. The exported decoder has **no KV cache**, so each step re-feeds the whole prefix.
  - Why is that acceptable? *(Formulas are short; a whole run is ~0.25–0.5 s.)*
- [ ] **`argmax_prob`**: greedy pick plus softmax probability of the winner. Check the maths: `1 / Σ exp(logit − max)`.
- [ ] **`looped`**: tail is a chunk of period 1–12 repeated ≥ 4 times and ≥ 32 tokens. Why those numbers? *(Real LaTeX repeats, e.g. `0&0&0`, but not 32 tokens exactly; see the test `detects_runaway_loops_but_not_legitimate_repetition`.)*
- [ ] **`LOW_CONFIDENCE = 0.5`**: chosen from data. At 0.5, 0 of 45 exact results flagged; at 0.7, 12 of 45.
- [ ] **`Stop`** (`Eos` / `MaxTokens` / `Looped`) and how `main.rs` reacts to each (looped → error exit, nothing copied).

### B. Model spec and downloading (`src/spec.rs`, `src/fetch.rs`)
- [ ] `ModelSpec` fields: file names, input size, mean/std, `fit`, start/EOS tokens, `base_url`. Anything omitted from `spec.json` keeps its default (`#[serde(default)]`).
- [ ] Model lookup order in `fetch::locate`: `--model-dir` → `./models` → per-user cache. Download happens only if nothing complete is found and `base_url` is set.
- [ ] Downloads write to a `.part` file and rename on success, so a half download is never mistaken for a model.
- [ ] Try it: make a folder with symlinks to the ONNX files and a `spec.json` of `{"name":"x","base_url":null}`, then run with `--model-dir`. Remove a file and confirm the error says what's missing.
- [ ] Required ONNX shape of any model: encoder `pixel_values → last_hidden_state`; decoder `input_ids + encoder_hidden_states → logits`, no KV cache (the Hugging Face `optimum` export layout).

### C. Capture and clipboard (`src/capture.rs`, `src/clipboard.rs`)
- [ ] `screenshot` uses `xcap` (whole monitor); it's behind the `capture` Cargo feature, on by default.
- [ ] `crop` parses `x,y,w,h` in screenshot pixels and clamps to bounds (2 tests).
- [ ] `interactive` is macOS-only (`screencapture -i`); other OSes get a message listing alternatives. The real cross-platform selector is **Phase 3**.
- [ ] Linux clipboard: `copy_text` spawns a detached copy of the program (`--hold-clipboard`) that keeps serving the text. *Compiles for Linux; has never run on Linux.*
- [ ] Windows/macOS use `arboard` directly.

### D. The tidy rules (`src/tidy.rs`)
Order in `tidy()`: `compact` → `modernize_fonts` → `known_operators` → `delimited_matrices` → `unwrap_matrix_cells` → `binomials` → `mathrm_to_text` → `unbrace_scripts` → drop trailing `\\` before `\end`.

- [ ] **`compact`**: removes all whitespace except the one space TeX needs between a control word and a letter (`\alpha x`). Real spaces survive because the model writes them as `~` or `\ `.
- [ ] **`modernize_fonts`**: `{\bf E}` → `\mathbf{E}`, keeping braces where they matter (`x^{\bf a}`, `\frac{a}{\bf b}`). See `group_is_removable` and the `TAKES_ARG` list.
- [ ] **`known_operators`**: `\operatorname{sin}` → `\sin` for names in `OPERATORS`.
- [ ] **`delimited_matrices`**: `\left(\begin{matrix}…\right)` → `pmatrix`; `[]` → `bmatrix`; `||` → `vmatrix`; `\|\|` → `Vmatrix`; `\{ \}` → `Bmatrix`; `\{ … \right.` → `cases`.
- [ ] **`unwrap_matrix_cells`**: strips the `{a}` wrapper around cells, but not when a cell is two groups like `{a}{b}`.
- [ ] **`binomials`**: `\left(n\atop k\right)` → `\binom{n}{k}` only with exactly one top-level `\atop` and a `)` closer.
- [ ] **`mathrm_to_text`**: `\mathrm{if~}` → `\text{if }` only when `~` or `\ ` shows it's prose; `\mathrm{Var}` stays.
- [ ] **`unbrace_scripts`**: `x^{2}` → `x^2` only for a single letter or digit.
- [ ] Output needs `amsmath` in the destination document (`\text`, `\binom`, `pmatrix`). `--raw` shows the model's untouched output.
- [ ] Safety property: the benchmark's re-render check proves tidy changes formatting, not appearance. Run `python3 tools/bench.py --verbose` and read the outputs yourself.

### D2. Prose mode (`src/prose.rs`)
Runs *before* the math rules: `tidy_with_kind` compacts the text, tries `prose::convert`, and falls back to the formula pipeline if it returns `None`.

- [ ] **Rows** (`rows`): strips a single-column `array` wrapper (`{l}`, `{c}`, `{r}`; anything else is a table and is rejected), splits at top-level `\\`, drops a `[-2pt]` spacing argument, strips redundant outer braces.
- [ ] **Pieces** (`scan`): `\mathrm`/`\text`/`\textup`... groups are text; `~`, `\;`, `\,`, `\ ` are spaces; `\vspace{..}`, `\hfill`, `\noindent` are dropped; other commands (`\overline{G}`) are math; outside a text group, runs of digits are text and everything else is math; trailing `. , ; :` always go back to text; `^`/`_` keep their argument verbatim (including inside a text group; this was a bug once).
- [ ] **Words → math** (`looks_math`, `explode_math_words`): a lone letter except `a`, `A`, `I`; words with `= < > + ^ _ [ ] \`; a digit with letters but no run of 3 letters (`n-1` yes, `3-regular` no).
- [ ] **Joining** (`repair_line_breaks`, `join_math`): `cut-` + `vertex`; neighbouring math, an operator and its operand (`n\geq` + `1`), a formula cut by a line break (`1+2+` / `\cdots+n`).
- [ ] **Deciding** (`is_prose`): at least 4 real words (3+ letters) and word-like tokens ≥ math-like tokens. Check the test `formulas_stay_formulas`.
- [ ] **Rendering** (`render`): space between words; none before punctuation; between a word and a formula a space unless the text side is punctuation or a hyphen (`$x$-axis`, `$G$.`).
- [ ] `has_run_together_words`: any 20+ letter run in running text means the model lost the spaces; the CLI warns.
- [ ] Real outputs are in the tests; two are reconstructed, not captured: the `\vspace` case and the glued `\textup` case (they are marked in comments).

### D3. Vision engine and setup (`src/vlm.rs`, `src/helper.rs`, `src/ui.rs`, `src/fetch.rs`)
- [ ] **Routing** (`main.rs::read`): vision first; real text -> prose (with `prose::mark_math`); a bare formula -> small model, with the vision formula only as a fallback if the small model loops; `--engine vlm` returns the vision result as is.
- [ ] **`interpret`**: prose if there is any real word next to math, or 3+ real words with no math. A one-line exercise like `Is $C_6$ bi-graceful?` must stay prose (an earlier version dropped its words; see the test `one_line_exercises_keep_their_words`).
- [ ] **Setup flow** (`vlm::ensure`): lists each item with size, source, licence and destination, asks once, then does `[1/2]`, `[2/2]` with progress. `--yes` proceeds; no terminal and no `--yes` is an error, never a silent yes.
- [ ] **Pinned and verified**: `helper::RELEASES` (llama.cpp `b11146`) and `vlm::MODEL_FILES` carry sizes and SHA-256s; `fetch::download_verified` deletes a file that fails (tests with a local HTTP server cover success, bad checksum, resume, a server that ignores `Range`, and an already-intact file).
- [ ] **Helper lookup** (`vlm::find_server`): `--llama-server`, else the copy texcopy installed, else PATH.
- [ ] **Process hygiene**: `Server` is killed on drop; no `llama-server` or `texcopy-llama-*.log` left behind after a run.
- [ ] **Memory**: started with `--cache-ram 0` (the default prompt cache grows from 1.4 GB past 3 GB).
- [ ] Try it yourself: `texcopy --setup`, answer `n` (nothing is created), then again with `y`.

### E. The command line (`src/main.rs`)
- [ ] Input options are mutually exclusive: file, `--clipboard`, `--screen` (with `--monitor`, `--region`), or none (macOS selector). Try the invalid combos and confirm the errors.
- [ ] Output options (`-w` is ignored for sentences): `-w none|inline|display`, `--no-copy`, `--raw`, `--json`, `--fit stretch|pad`, `--max-tokens`, `--model-dir` (also `TEXCOPY_MODEL_DIR`).
- [ ] Warnings go to stderr, the result to stdout, so `texcopy x.png > out.tex` stays clean.
- [ ] `--json` never touches the clipboard.

### F. The benchmark (`tools/bench.py`)
- [ ] How a case is graded: render true LaTeX → run texcopy → compile its output → compare pixels (`magick compare -metric AE`). **EXACT** = identical, **close** = differs but ≥ 90% text similarity, **differs**, **FAIL** = doesn't compile.
- [ ] Suites: `basic` (T1–T4), `degraded` (dpi, blur, dark, JPEG, noise, margins, fonts), `stress` (wide, aligned rows, matrices, rare symbols), `prose` (your screenshot plus five sentences at four sizes, graded as text with spacing mattering outside `$...$`). Table and paragraph are **info only**.
- [ ] Cases that can't be rendered are printed as `SKIPPED` (they used to be dropped silently; see section 5).
- [ ] `--baseline` diffs per-case grades; `--save` records; `--tool-args "--fit pad"` passes flags through.
- [ ] Confidence report at the end of each run: how well the least-sure token predicts a wrong answer (~0.82 separation, where 0.5 is useless and 1.0 perfect).
- [ ] Limits: 57 cases, mostly textbook formulas, so treat the numbers as directional.

### G. Build, release and CI
- [ ] `Cargo.toml` release profile: `lto = "fat"`, `panic = "abort"`, `strip = true`. Why not `opt-level = "s"`/`"z"`? They saved 1–2 MB but made 4K captures 13–57% slower.
- [ ] Size breakdown: ~78% of machine code is ONNX Runtime (C++); Rust is ~21%. So compiler flags have limited reach.
- [ ] `image` crate trimmed to PNG, JPEG, WebP, BMP, GIF (TIFF comes in via `arboard`). Others, such as TGA, give "format not supported".
- [ ] `tokenizers` uses the pure-Rust regex feature (`unstable_wasm`), so there's no `onig` C library. Only `decode` is used, and the benchmark confirmed identical results.
- [ ] `ci.yml`: matrix `macos-14` (Apple Silicon), `windows-latest`, `ubuntu-latest`, with the Linux capture libraries installed, plus a `--no-default-features` job. **It has never run** (no GitHub remote).
- [ ] Intel Macs are deliberately unsupported (no prebuilt ONNX Runtime for them in this `ort` version).

### H. Docs
- [ ] `README.md` sections present: How it works, Using a different model, Usage, **Build**, Benchmark, Limitations. (The Build section was once deleted by accident; confirm it's there.)

---

## 3. Known gaps and unverified claims

Resolve these over time; tick when closed.

- [ ] Windows/Linux: the llama.cpp helper install (zip / tar.gz unpack, CPU build speed) and the vision engine were only run on Apple Silicon.
- [ ] macOS: the first run after installing the helper took 19 s (system vetting of a new program); later runs 1 s. Confirm this on your Mac.
- [ ] Windows: run the tool on a real Windows machine (clipboard, `--screen`, cache folder location).
- [ ] Linux: run on X11 *and* Wayland; confirm the clipboard helper keeps text after the CLI exits; confirm `xcap` works.
- [ ] Push to GitHub and get all three CI jobs green (or fix them).
- [ ] `--screen` on macOS: confirm it captures real screen contents, not a blank image from missing Screen Recording permission.
- [ ] Multi-line formulas: the model can merge lines (e.g. `aligned` → `…\,…`) *with high confidence*. Confidence does not catch this.
- [ ] A larger, less textbook benchmark (real paper screenshots, tables of different fonts).
- [ ] The model reads some symbols wrongly with confidence (e.g. `\nexists` as `\exists`).
- [ ] Words without explicit spaces in `\mathrm{…}` stay `\mathrm` (can't tell operator from prose).
- [ ] Prose: three or more lines lose the spaces between words (warned, not fixed). Fix would be splitting the image into lines and reading each; also would help wide single lines.
- [ ] Prose: italic `v` is sometimes read as `\nu` (4 of the 21 prose cases); the CLI notes it but can't resolve it. `a`/`A`/`I` are always treated as words.
- [ ] Math fonts: only Times, txfonts and Latin Modern could be tested (all exact). Palatino, Utopia, Charter etc. render blank on this machine's TeX install, so they are untested, not failing.
- [ ] `tools/fixtures/textbook-5.4.png` is a screenshot from a textbook. Decide whether to keep it in the repo before publishing it.
- [ ] `\vspace` and glued-`\textup` unit tests use reconstructed model output, not verbatim captures.

---

## 4. Decisions and why

| Decision | Why |
|---|---|
| Local ONNX model, no server | Privacy and offline use |
| Keep `pix2text-mfr` for now | Near-perfect on clean formulas; alternatives need conversion or have licence doubts. Revisit later |
| Stretch resize default | Matches the model's training preprocessing; wins on wide formulas |
| Confidence warning at 0.5 | Zero false alarms on exact results in the benchmark |
| `ModelSpec` / `spec.json` | Swap models, or use one you train, without code changes |
| Supported: Windows x64, macOS Apple Silicon, Linux x64 | Your call; Intel Macs dropped |
| Popup app will be egui, on-demand (no tray) | Pure Rust, one window toolkit, you bind your own shortcut |
| One-line conventional commits, no trailers | Your standing preference |

---

## 5. Mistakes made (read these parts twice)

- [ ] A stray test run downloaded a second model copy into the user cache (deleted, and the cause was running from the wrong directory).
- [ ] Benchmark bug: wide formulas were re-rendered on a narrower page than the reference. Fixed in `76b4fb3`; the `baseline.json` is from the corrected harness.
- [ ] README Build section silently deleted in step 3; restored in `5511c2f`.
- [ ] First four commits had bodies and a co-author trailer; rewritten with identical contents.
- [ ] One test of mine expected `^{n}` where the code correctly produced `^n` (test fixed, code was right).
- [ ] The benchmark's **font cases never ran** for most of the project: `-halt-on-error` treated a harmless missing-font warning as failure and the harness skipped them silently. Now they run, reference renders use a tolerant mode, and skips are printed. Result: Times passes, Palatino fails.
- [ ] **I then reported a false weakness.** After the font cases ran, Palatino "failed" for every model, and I wrote it up as a real limit of the model. The reference image was actually 25x25 pixels and blank (missing font metrics dropped every glyph). The harness now rejects blank references, and the claim is removed from the README and this checklist.
- [ ] Prose grading first ignored all whitespace, which hid lost spaces (a glued 3-line paragraph graded "close"). Now spacing outside `$...$` counts.
- [ ] Prose mode first dropped the braces in `e^{i\pi}` inside a text group (found by looking at the gallery, not by a test). Fixed and tested.

---

## 6. Commit map

| Commit | What |
|---|---|
| `76aae43` | Initial CLI |
| `ca2742a` | Benchmark + baseline |
| `c692c63` | `ModelSpec` refactor |
| `fc4cbab` | Portable capture, cache dir, clipboard |
| `9cf17f1` | Stretch default, `--fit` |
| `be4d574` | Confidence, loop detection, `--json` |
| `1559bae` | Tidy rules: matrices, binom, text |
| `76b4fb3` | Benchmark same-page fix |
| `8c87b20` | amsmath note |
| `5f670ed` | Clippy fixes |
| `e6de0a4` | rustfmt |
| `d4b654a` | CI workflow |
| `9de93d4` | Pure-Rust regex in `tokenizers` |
| `4708deb` | Fat LTO, `panic=abort`, fewer codecs |
| `5511c2f` | README Build section restored |
| `b636caa` | This checklist |
| `5284381` | Visual gallery tool |
| `c48dea5` | Prose suite + textbook fixture in the benchmark |
| `84e74b6` | Prose mode (`src/prose.rs`) |
| `526a97d` | Strict prose grading, font cases run, refreshed baseline |
| `9fcf054` | Textbook-problem cards in the gallery |

`git show <hash> --stat` shows each one's footprint.

---

## 7. Roadmap

- [x] **Phase 1** — portable core, quality, CI
- [x] **Phase 2** — spike (`spikes/gui/`): a full-screen overlay turned into a popup was fragile (a tiling window manager moved the windows, a second window was offset by the menu bar). Result: use the OS's own selector where there is one, and run texcopy's overlay as its own process otherwise.
- [x] **Phase 3** — the window (`src/gui.rs`): Capture button, wrap and engine options, output text; in-window first-run download with progress; the vision engine is pre-warmed while you drag and unloaded when idle; settings remembered.
- [x] **Phase 4** — packaging (`packaging/`): macOS `.app`, Windows zip with a console-free `texcopy-gui.exe`, Linux tarball with installer, release workflow. **Never run on Windows or Linux yet** (see the checklist below).
- [x] **Decided:** vision-language backend OvisOCR2 via a `llama-server` helper (the Rust-native `oar-ocr-vl`/Candle route was spiked and rejected: 4.7 s on CPU, 72-78 s on Metal, 3.7-4.9 GB, vs 0.3 s and 1.4 GB). Context: (OvisOCR2 / MinerU2.5-Pro / PaddleOCR-VL). On 45 real textbook exercises: texcopy's small model 22 exact / 14 close / 9 wrong; all three vision models 44 exact / 1 close / 0 wrong, ~0.3-0.45 s each, ~1.35-1.5 GB RAM while loaded (with `--cache-ram 0`), 1.0-1.8 GB on disk.
- [ ] **Next candidate** — split images into text lines and read each (fixes 3+ line paragraphs and very long lines); decide how to handle `v` vs `\nu`
- [ ] **Later** — train or fine-tune your own model (the data can be generated with `pdflatex`); minimal ONNX Runtime build for a much smaller binary; row-splitting for multi-line formulas; splitting very wide equations

### Review checklist for phases 3 and 4

- [ ] **`src/engine.rs`**: `Options` (what to do) and `Warm` (the loaded engines). The command line makes a fresh `Warm` per run; the window keeps one. Check `prewarm` never downloads or asks.
- [ ] **`src/gui.rs`**: three threads (UI, one worker owning the engines, a short thread per Capture). Closing the window must join the worker, or `llama-server` is orphaned. `vlm.rs` also records each helper's pid in a note file and reaps helpers whose owner died (macOS and Linux).
- [ ] **`src/convert.rs`**: the model's Markdown to LaTeX: headings, lists, display equations, paragraphs, code, and one cleanup path for text. **`src/table.rs`**: HTML tables (and plain-line tables) to `tabular`. Both are graded by the `documents` and `tables` suites in `tools/bench.py`, which re-render the output (documents) or compare cells (tables).
- [ ] **Output is "a correct reconstruction", not the author's source.** Bold/italic/small caps, vertical table rules and theorem styling are not in the model's output and are out of scope.
- [ ] **`src/capture.rs`, `src/overlay.rs`**: selector order is the OS's own (macOS `screencapture`, Linux tools), then the overlay. The overlay is verified on macOS only (`TEXCOPY_SELECTOR=overlay`, `TEXCOPY_OVERLAY_AUTO` for a scripted drag); the Linux tool chain is tested with fake programs.
- [ ] **Not yet verified on a real machine:** the Windows and Linux builds, the overlay on Windows/Linux/Wayland/multi-monitor, the release workflow (CI has never run: there is no GitHub remote yet). First thing to do after pushing: run the CI and the release workflow by hand ("Run workflow") and fix what they report.
- [ ] **Signing:** the macOS app is ad hoc signed, not notarised; Windows is unsigned. Users see an "unidentified developer" / SmartScreen warning once (the README says how to pass it).

---

## 8. Glossary

- **Encoder / decoder**: the two networks; the first reads the image, the second writes tokens.
- **Token**: a chunk of text the model emits (`\`, `frac`, `{`, `x`…); this model knows 1,200 of them.
- **Greedy decoding**: always take the likeliest next token.
- **Logit / softmax**: raw score per token / the function that turns scores into probabilities.
- **KV cache**: a speed trick the exported decoder doesn't have, so it re-reads its whole output each step.
- **ONNX / ONNX Runtime**: a portable network file format / the engine that runs it.
- **LTO**: link-time optimization, whole-program optimization across crates.
- **EXACT / close / differs / FAIL**: the benchmark's grades (section 2F).
- **Frozen-screenshot overlay**: texcopy's own selector (`src/overlay.rs`): capture the screen first, then draw a dimmed copy to drag over. Used where the OS has no selector of its own.
