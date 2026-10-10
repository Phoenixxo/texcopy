# texcopy

Drag-select anything on screen that was rendered with LaTeX and get its LaTeX source on your clipboard.

```
$ texcopy                      # opens the window: click Capture, drag over a formula
\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}
Copied to clipboard.
```

Everything runs locally. Nothing is uploaded.

## The window

`texcopy` with no arguments opens a small window: a **Capture** button (or press `C`), the options, and the output text. Click Capture, drag over something, and the LaTeX appears and is copied to your clipboard. Changing `wrap` or `engine` updates it; Esc closes the window.

- **On macOS the selection is the system crosshair** (`screencapture -i`), so tiling window managers and multiple monitors cannot get in the way. Windows and Linux do not have a selector in the window yet; use `texcopy --clipboard` after a Snipping Tool or Flameshot capture.
- **First run:** if the vision engine is not set up, the window offers to download it (size shown, checksum-verified, with a progress bar). "Not now" is remembered.
- **It stays fast:** the window keeps the vision engine loaded between captures (the second read takes about 0.3 s instead of 1-2 s) and unloads it after 2.5 minutes idle, freeing about 1.4 GB.
- Settings (`wrap`, `engine`) are remembered in `settings.json` in the per-user config folder.
- Notes appear in small grey text under the output only when something deserves a second look (low-confidence symbols, a possible cut-off, run-together words).

## How it works

LaTeX output (a screenshot, a PDF page) doesn't contain its source, so recovering it is OCR:

1. The image comes from a file, the clipboard, a screenshot (`--screen`, cross-platform via `xcap`) or, on macOS, `screencapture -i` drag-select.
2. The image is flattened onto white, inverted if it's dark mode, and resized to the model's 384x384 input (stretched by default, which handles wide formulas best; `--fit pad` keeps the aspect ratio instead).
3. A vision transformer (the *encoder*) turns it into 578 feature vectors.
4. A small transformer *decoder* writes LaTeX tokens one at a time, looking at those vectors, until it emits end-of-sequence.
5. `tidy.rs` rewrites the token stream (`\frac { a } { b }`) into idiomatic LaTeX (`\frac{a}{b}`). It only changes formatting, never meaning. If the image turns out to be a sentence rather than a formula, `prose.rs` takes over (see below).

**Two engines.** The *small* engine is described below. The *vision* engine (OvisOCR2, a 0.8B vision-language model) reads whole paragraphs and very long equations, which the small one cannot; see "Vision engine". `--engine auto` (the default) uses the vision engine when it is set up, and for a bare formula still prefers the small model.

The small model is [`breezedeus/pix2text-mfr`](https://huggingface.co/breezedeus/pix2text-mfr) (MIT), run through ONNX Runtime via the [`ort`](https://crates.io/crates/ort) crate. About 118 MB, downloaded on first run to your OS's per-user cache folder (for example `~/Library/Caches/texcopy` on macOS). A `./models` directory or `--model-dir` / `$TEXCOPY_MODEL_DIR` takes precedence.

## Sentences with inline math

Point it at a line or two of a textbook problem and it returns running text with the math marked:

```
$ texcopy problem.png
5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.
```

With the vision engine, a page of text works too. texcopy rebuilds it as LaTeX that **renders the same as the picture**; it cannot know the author's original source (the picture can't tell `\textbf{x}` from `{\bf x}`), so the result is a correct reconstruction, not a copy:

```
$ texcopy page.png
\section*{Introduction}
Today I am learning \LaTeX{}. I can write in line math such as $a^2 + b^2 = c^2$.

I can also give equations their own space:
\begin{equation}
\gamma^{2}+\theta^{2}=\omega^{2}
\end{equation}
```

- **Display equations** are display equations. With a number beside them (`(1)`, or a `\tag`) they become `\begin{equation}...\end{equation}` (LaTeX writes the number itself); without one they become `\[ ... \]`; `align`-type environments are kept as they are. A page number left at the very end after one is dropped.
- **Paragraphs** keep their blank line; lines wrapped inside a paragraph are joined.
- **Headings** (`##`) become `\section*{...}` (`###` is `\subsection*`). The model marks centred text as a heading too, and texcopy can't tell the difference.
- **Lists** become `itemize` / `enumerate`, nested when the bullets change. A lone "5." is a problem number, not a list.
- **Text is made safe for LaTeX:** `& % # _` and prices (`\$5`) are escaped, `"..."` and curly quotes become ` ``...'' `, dashes become `--` and `---`, a `□` ends a proof flush right, and the drawn `LATEX`/`TEX` logos become `\LaTeX{}`/`\TeX{}`.
- **Code** in a fenced block becomes `verbatim`; `**bold**` and `*italic*` Markdown, if the model writes any, become `\textbf` and `\emph`.
- **Not recoverable:** the model usually returns bold, italic, small caps, underline and typewriter text as plain words, and does not report vertical table rules, so those come out plain.

**Tables** (vision engine) come back as a `tabular`:

```
\begin{center}
\begin{tabular}{rcc}
\hline
$n$ & Recursive time (ms) & DP time (ms) \\
\hline
5 & 0.000037 & 0.000022 \\
...
\hline
\end{tabular}
\end{center}
```

The model reads the cells; the layout is guessed, because the model's HTML table says nothing about alignment or rules. Number-like columns (`12`, `0.5`, `$1,250.00`, `50%`) are right-aligned and text columns left-aligned; there is a rule above, below, and under a header row, and no vertical lines. `colspan` becomes `\multicolumn`; `rowspan` leaves the covered cells empty (which row of the span the text belongs in is ambiguous). A single letter in the header row (`n`) is a variable, so it becomes `$n$`; in the body it is a label (a grade `C`) and stays text. Cells are cleaned like any other text: `&`, `%`, `_` and prices are escaped. Adjust `{lrr}` and the rules to taste.

Sometimes the model writes a table as plain lines instead (`Id Name Year` / `1 Ann 2021` ...). Three or more lines of the same number of short words with a column of numbers are rebuilt as a table too; a line ending in a colon is a caption, not a row.

The tables benchmark (`tools/bench.py --suite tables`) grades the *cells*, not the look: 16 of 18 exact. The two misses are a `rowspan` whose text sits in the other row, and `g/cm$^3$` that the model reads as `g/cm3`.

## How it works

LaTeX output (a screenshot, a PDF page) doesn't contain its source, so recovering it is OCR:

1. The image comes from a file, the clipboard, a screenshot (`--screen`, cross-platform via `xcap`) or, on macOS, `screencapture -i` drag-select.
2. The image is flattened onto white, inverted if it's dark mode, and resized to the model's 384x384 input (stretched by default, which handles wide formulas best; `--fit pad` keeps the aspect ratio instead).
3. A vision transformer (the *encoder*) turns it into 578 feature vectors.
4. A small transformer *decoder* writes LaTeX tokens one at a time, looking at those vectors, until it emits end-of-sequence.
5. `tidy.rs` rewrites the token stream (`\frac { a } { b }`) into idiomatic LaTeX (`\frac{a}{b}`). It only changes formatting, never meaning. If the image turns out to be a sentence rather than a formula, `prose.rs` takes over (see below).

**Two engines.** The *small* engine is described below. The *vision* engine (OvisOCR2, a 0.8B vision-language model) reads whole paragraphs and very long equations, which the small one cannot; see "Vision engine". `--engine auto` (the default) uses the vision engine when it is set up, and for a bare formula still prefers the small model.

The small model is [`breezedeus/pix2text-mfr`](https://huggingface.co/breezedeus/pix2text-mfr) (MIT), run through ONNX Runtime via the [`ort`](https://crates.io/crates/ort) crate. About 118 MB, downloaded on first run to your OS's per-user cache folder (for example `~/Library/Caches/texcopy` on macOS). A `./models` directory or `--model-dir` / `$TEXCOPY_MODEL_DIR` takes precedence.

## Sentences with inline math

Point it at a line or two of a textbook problem and it returns running text with the math marked:

```
$ texcopy problem.png
5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.
```

With the vision engine, a page of text works too. texcopy rebuilds it as LaTeX that **renders the same as the picture**; it cannot know the author's original source (the picture can't tell `\textbf{x}` from `{\bf x}`), so the result is a correct reconstruction, not a copy:

```
$ texcopy page.png
\section*{Introduction}
Today I am learning \LaTeX{}. I can write in line math such as $a^2 + b^2 = c^2$.

I can also give equations their own space:
\begin{equation}
\gamma^{2}+\theta^{2}=\omega^{2}
\end{equation}
```

- **Display equations** are display equations. With a number beside them (`(1)`, or a `\tag`) they become `\begin{equation}...\end{equation}` (LaTeX writes the number itself); without one they become `\[ ... \]`; `align`-type environments are kept as they are. A page number left at the very end after one is dropped.
- **Paragraphs** keep their blank line; lines wrapped inside a paragraph are joined.
- **Headings** (`##`) become `\section*{...}` (`###` is `\subsection*`). The model marks centred text as a heading too, and texcopy can't tell the difference.
- **Lists** become `itemize` / `enumerate`, nested when the bullets change. A lone "5." is a problem number, not a list.
- **Text is made safe for LaTeX:** `& % # _` and prices (`\$5`) are escaped, `"..."` and curly quotes become ` ``...'' `, dashes become `--` and `---`, a `□` ends a proof flush right, and the drawn `LATEX`/`TEX` logos become `\LaTeX{}`/`\TeX{}`.
- **Code** in a fenced block becomes `verbatim`; `**bold**` and `*italic*` Markdown, if the model writes any, become `\textbf` and `\emph`.
- **Not recoverable:** the model usually returns bold, italic, small caps, underline and typewriter text as plain words, and does not report vertical table rules, so those come out plain.

**Tables** (vision engine) come back as a `tabular`:

```
\begin{center}
\begin{tabular}{rcc}
\hline
$n$ & Recursive time (ms) & DP time (ms) \\
\hline
5 & 0.000037 & 0.000022 \\
...
\hline
\end{tabular}
\end{center}
```

The model reads the cells; the layout is guessed, because the model's HTML table says nothing about alignment or rules. Whole-number columns are right-aligned and the rest centred; there is a rule above, below, and under a header row, and no vertical lines. `colspan` becomes `\multicolumn`; `rowspan` leaves the covered cells empty. Adjust `{rcc}` and the rules to match your table.

The model is trained on formulas, so it writes sentences as `\mathrm{W o r d s ~ l i k e ~ t h i s}`, wraps visual lines in an `array`, and doesn't say which letters were italic. Prose mode (`src/prose.rs`) flattens the lines into one paragraph, glues words broken at a line end (`cut-` + `vertex`), re-joins a formula cut by a line break, and wraps the math in `$...$` using rules: a lone letter other than `a`, `A`, `I` is math; so are intervals like `[a,b]`, expressions with `= < > + ^ _`, and digit-letter mixes like `n-1` (but not `3-regular`). It only activates when there are at least four real words and no more math-like tokens than word-like ones, so formulas with a bit of `\text{if }` are untouched. `--wrap` is ignored for sentences, and `--json` reports `"kind": "prose"` or `"math"`.

Limits, measured on `tools/bench.py --suite prose` (21 cases, 17 exact):

- **One or two lines work; three or more are unreliable.** The model reads the whole image at once and on a tall block it can lose the spaces between words (`beaconnectedgraphwith`). texcopy warns when it sees that. Select one line at a time.
- **It can't see italics**, so it cannot tell the variable `a` from the article "a" (both stay words), and it sometimes reads an italic `v` as `\nu`. texcopy prints a note whenever it outputs `\nu`.
- Wording errors by the model (`Euler` read as `Enler`) are flagged by the low-confidence warning when it was unsure.

## Vision engine (recommended for text)

On 45 real exercises from a graph-theory textbook the small model gets 22 exactly right; the vision engine gets 44 (one more differs only by a dropped full stop). It also reads 3-6 line paragraphs and equations with 20+ terms that the small model garbles.

```
texcopy --setup             # one-time: asks first, then downloads, with progress
texcopy problem.png         # --engine auto uses it from now on
texcopy --engine vlm x.png  # force it (sets itself up if needed)
```

`--setup` explains what it will fetch and asks once (`--yes` answers for you; without a terminal it refuses rather than assume):

1. **llama.cpp `llama-server`** (MIT licence), release `b11146`, 11-19 MB, from github.com/ggml-org/llama.cpp. The release and the SHA-256 of each platform's file are pinned in `src/helper.rs`; a download that doesn't match is deleted. macOS Apple Silicon gets the Metal (GPU) build; Windows and Linux get the CPU build. For a GPU build there, install one yourself and pass `--llama-server`. An existing `llama-server` on your PATH is used if texcopy has not installed its own.
2. **OvisOCR2** (Apache-2.0), 1.02 GB (an 8-bit model file and its vision part) from huggingface.co/bartowski/ATH-MaaS_OvisOCR2-GGUF, checksum-verified.

Both go in texcopy's cache folder (`TEXCOPY_CACHE_DIR` overrides it). Downloads show a progress bar with speed and time left, and an interrupted one resumes where it stopped.

How it runs: each texcopy run starts `llama-server` on a free local port, reads the image, and stops it again, so nothing stays in memory between runs. On an Apple M4 Pro that is about 1 second end to end, with about 1.4 GB of RAM while it is running. The first run after installing can take longer while macOS checks the new helper. Everything stays on your machine.

Details: the vision engine does not report per-token confidence, so the "unsure about" warnings only exist for the small model. Its text gets the same math-marking rules as prose mode (lone letters like `v` become `$v$`).

## Using a different model

Model-specific values live in `src/spec.rs`. To use another encoder-decoder, or one you trained yourself, put its ONNX files in a folder and add an optional `spec.json` (any omitted field keeps the pix2text-mfr default):

```json
{
  "name": "my-model",
  "encoder": "encoder_model.onnx",
  "decoder": "decoder_model.onnx",
  "tokenizer": "tokenizer.json",
  "input_width": 384, "input_height": 384,
  "mean": [0.5, 0.5, 0.5], "std": [0.5, 0.5, 0.5],
  "start_token": 2, "eos_token": 2,
  "base_url": null
}
```

then run `texcopy --model-dir path/to/folder`. The ONNX graphs must follow the Hugging Face `optimum` export convention: encoder `pixel_values` -> `last_hidden_state`; decoder `input_ids` + `encoder_hidden_states` -> `logits`, without a KV cache. Set `base_url` to a place that serves `{base_url}/{file}` if texcopy should download missing files, or `null` for local-only models.

## Usage

```
texcopy                     # open the window (Capture button, options, output text)
texcopy --select            # no window: drag-select a screen region in the terminal (macOS; other OSes: see below)
texcopy formula.png         # from a file
texcopy --clipboard         # from an image on the clipboard
texcopy --screen --region 100,200,600,150   # capture the primary monitor, crop to x,y,w,h (screenshot pixels)
texcopy --screen --monitor 1                # a specific monitor
texcopy -w inline           # wrap in $...$   (-w display: \[ ... \])
texcopy --no-copy           # print only
texcopy --raw               # skip tidy, show the model's raw tokens
texcopy --fit pad           # pad to the model's aspect ratio instead of stretching
texcopy --json              # latex + engine + stop reason (+ per-token confidence for the small model) as JSON, no clipboard
texcopy --setup             # set up the vision engine (asks first), then exit
texcopy --engine small|vlm|auto   # which engine reads the image (default auto)
texcopy --yes               # answer yes to setup questions
```

Targets: Windows x64, macOS on Apple Silicon, Linux x64 (X11; Wayland best-effort). Drag-to-select is built in on macOS only for now; a cross-platform selector window is planned. Until then, on Windows and Linux use `--screen --region`, a file, or `--clipboard` with your OS's snipping tool.

macOS asks for Screen Recording permission for your terminal the first time you capture the screen.

**Linux clipboard:** X11 and Wayland serve the clipboard from the program that set it, so texcopy leaves a small background helper running that keeps the text available until you copy something else.

## Build

```
cargo build --release       # binary at target/release/texcopy
cargo test
cargo build --release --no-default-features   # command line only: no window, no screen capture (fewer system dependencies)
cargo build --release --no-default-features --features capture   # command line + screen capture, no window
```

The first build downloads a prebuilt ONNX Runtime library. The Rust dependencies need no extra C library or compiler setup beyond the platform's standard toolchain.

Screen capture (the default `capture` feature) needs these packages on Linux:

```
apt-get install pkg-config libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev
```

The release binary is about 31 MB with the window (30.8 MB on Apple Silicon; about 23 MB without it), built with fat LTO, `panic = "abort"` and stripped symbols. About four fifths of its machine code is ONNX Runtime (C++), statically linked, so Rust compiler flags can only shrink the remaining fifth. Optimizing for size (`opt-level = "s"`/`"z"`) would save another 1-2 MB but made full-screen captures 13-57% slower, so it is not used. Image files in PNG, JPEG, WebP, BMP, GIF and TIFF are supported.

CI (`.github/workflows/ci.yml`) builds, lints and tests on macOS (Apple Silicon), Windows and Linux.

## Benchmark

`tools/bench.py` renders known LaTeX with `pdflatex`, runs texcopy on the image, compiles the output and compares the two renders pixel by pixel. It needs `pdflatex`, `pdftoppm` and ImageMagick, and is a dev tool only.

```
python3 tools/bench.py                                   # all suites
python3 tools/bench.py --baseline tools/baseline.json    # report what got better or worse
```

`tools/book_cases.py` cuts real exercise screenshots out of a textbook PDF that has a text layer (ebook conversions work well) and writes ground truth from the PDF's own text and italics. Put them in the gitignored `tools/fixtures/private/` and run `python3 tools/bench.py --suite real`; the crops are copyrighted, so they are never committed. `tools/bakeoff.py` scores vision-language OCR models served by llama.cpp on the same cases (see `docs/REVIEW.md`).

## Limitations

- **Proofread the output.** texcopy warns about tokens the model was under 50% sure of, and refuses to copy output where the model got stuck repeating itself, but a confident answer can still be wrong. It is a small model. Typical mistakes: `\epsilon` read as `e`, `\bar` as `\vec`, `\text{...}` as `\mathrm{...}`.
- The cleaned-up output uses `\text`, `\binom` and `pmatrix`-style environments, so the document you paste into needs `amsmath` (use `--raw` to see exactly what the model produced).
- One formula or one or two lines of text per selection. Longer paragraphs, tables and multi-line alignments are out of scope. Tables sometimes come out nearly right, but don't rely on it.
- **Fonts:** Times, txfonts and Latin Modern math all read exactly in the benchmark. Other math fonts (Palatino, Utopia, Charter...) are untested, because the TeX installation the benchmark runs on doesn't ship them.
- The vision engine's helper install and its CPU speed on Windows and Linux are untested (everything was run on one Apple Silicon Mac). The pinned downloads exist for Windows x64/arm64, Linux x64/arm64 and macOS.
- Interactive drag-select is macOS-only for now. Only macOS has actually been run: the Windows and Linux clipboard and capture code type-checks against those targets but has not been exercised on a real machine.
- The model's vocabulary is only 1,200 tokens, so custom macros and rarely used symbols can't be represented.

If the source is a web page using KaTeX or MathJax, the exact TeX is already in the page's DOM and no OCR is needed.
