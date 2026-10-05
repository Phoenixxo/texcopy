# texcopy review checklist

Come back to this any time to remember what exists, why, and how to check it. Tick boxes as you go (`- [x]`). Everything here describes the state after **Phase 1** (git history up to `5511c2f`).

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
| `src/clipboard.rs` | read image / write text; Linux helper process | ~75 |
| `src/tidy.rs` | raw tokens → idiomatic LaTeX (9 unit tests) | ~625 |
| `tools/bench.py` | regression benchmark (needs `pdflatex`, `pdftoppm`, `magick`) | ~255 |
| `tools/baseline.json` | saved benchmark results to compare against | |
| `.github/workflows/ci.yml` | build + lint + test on macOS ARM, Windows, Linux | |

---

## 1. Ten-minute health check

Run from the repo root. Tick each when it behaves as stated.

- [ ] `cargo test --release` → **17 passed** (tidy 9, model 4, capture 2, fetch 1, spec 1)
- [ ] `cargo clippy --locked --all-targets -- -D warnings` → no warnings
- [ ] `cargo fmt --check` → no output
- [ ] `cargo build --release && ls -l target/release/texcopy` → about **22.9 MB** on Apple Silicon
- [ ] `python3 tools/bench.py --baseline tools/baseline.json` → totals **48 exact / 5 close / 3 differs / 1 FAIL**, and "**0 improved, 0 regressed**" (takes about a minute; needs the LaTeX tools)
- [ ] `./target/release/texcopy some-formula.png --no-copy` prints LaTeX
- [ ] `./target/release/texcopy some-formula.png --json` shows `latex`, `raw`, `stop`, `min_prob`, `tokens[]`

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

### E. The command line (`src/main.rs`)
- [ ] Input options are mutually exclusive: file, `--clipboard`, `--screen` (with `--monitor`, `--region`), or none (macOS selector). Try the invalid combos and confirm the errors.
- [ ] Output options: `-w none|inline|display`, `--no-copy`, `--raw`, `--json`, `--fit stretch|pad`, `--max-tokens`, `--model-dir` (also `TEXCOPY_MODEL_DIR`).
- [ ] Warnings go to stderr, the result to stdout, so `texcopy x.png > out.tex` stays clean.
- [ ] `--json` never touches the clipboard.

### F. The benchmark (`tools/bench.py`)
- [ ] How a case is graded: render true LaTeX → run texcopy → compile its output → compare pixels (`magick compare -metric AE`). **EXACT** = identical, **close** = differs but ≥ 90% text similarity, **differs**, **FAIL** = doesn't compile.
- [ ] Suites: `basic` (T1–T4), `degraded` (dpi, blur, dark, JPEG, noise, margins, fonts), `stress` (wide, aligned rows, matrices, rare symbols). Table and paragraph are **info only**.
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

- [ ] Windows: run the tool on a real Windows machine (clipboard, `--screen`, cache folder location).
- [ ] Linux: run on X11 *and* Wayland; confirm the clipboard helper keeps text after the CLI exits; confirm `xcap` works.
- [ ] Push to GitHub and get all three CI jobs green (or fix them).
- [ ] `--screen` on macOS: confirm it captures real screen contents, not a blank image from missing Screen Recording permission.
- [ ] Multi-line formulas: the model can merge lines (e.g. `aligned` → `…\,…`) *with high confidence*. Confidence does not catch this.
- [ ] A larger, less textbook benchmark (real paper screenshots, tables of different fonts).
- [ ] The model reads some symbols wrongly with confidence (e.g. `\nexists` as `\exists`).
- [ ] Words without explicit spaces in `\mathrm{…}` stay `\mathrm` (can't tell operator from prose).

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

`git show <hash> --stat` shows each one's footprint.

---

## 7. Roadmap

- [x] **Phase 1** — portable core, quality, CI
- [ ] **Phase 2** — spike: one `egui` window from full-screen frozen overlay to small popup (**check in after it**)
- [ ] **Phase 3** — popup app: drag-select overlay, editable result, copy button, wrap selector, low-confidence highlighting, background model load with progress bar
- [ ] **Phase 4** — packaging: shortcut docs per OS, macOS `.app`, release builds from CI
- [ ] **Later** — train or fine-tune your own model (the data can be generated with `pdflatex`); minimal ONNX Runtime build for a much smaller binary; row-splitting for multi-line formulas; splitting very wide equations

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
- **Frozen-screenshot overlay**: planned selector design; capture the screen first, then draw a dimmed copy to drag over, so it behaves the same on every OS.
