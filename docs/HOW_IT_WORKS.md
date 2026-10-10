# How texcopy works

The short version is in the [README](../README.md#-how-it-works). This page is the long one, for when you want to know what happens under the hood or want to change it.

## The small model's pipeline

LaTeX output (a screenshot, a PDF page) doesn't contain its source, so recovering it is OCR. This is the path through the small formula model; the vision engine, described below, replaces steps 2-4 with a single call to a larger model.

1. The image comes from a file, the clipboard, a screenshot (`--screen`, cross-platform via `xcap`) or a drag-selected region (see *Choosing a region* below).
2. The image is flattened onto white, inverted if it's dark mode, and resized to the model's 384x384 input (stretched by default, which handles wide formulas best; `--fit pad` keeps the aspect ratio instead).
3. A vision transformer (the *encoder*) turns it into 578 feature vectors.
4. A small transformer *decoder* writes LaTeX tokens one at a time, looking at those vectors, until it emits end-of-sequence.
5. `tidy.rs` rewrites the token stream (`\frac { a } { b }`) into idiomatic LaTeX (`\frac{a}{b}`). It only changes formatting, never meaning. If the image turns out to be a sentence rather than a formula, `prose.rs` takes over (see *Small-model prose mode* below).

**Two engines.** The *small* engine is described first. The *vision* engine (OvisOCR2, a 0.8B vision-language model) reads whole paragraphs and very long equations, which the small one cannot; see *The vision engine* below. `--engine auto` (the default) uses the vision engine when it is set up, and for a bare formula still prefers the small model.

The small model is [`breezedeus/pix2text-mfr`](https://huggingface.co/breezedeus/pix2text-mfr) (MIT), run through ONNX Runtime via the [`ort`](https://crates.io/crates/ort) crate. About 118 MB, downloaded on first run to your OS's per-user cache folder (for example `~/Library/Caches/texcopy` on macOS). A `./models` directory or `--model-dir` / `$TEXCOPY_MODEL_DIR` takes precedence.

## Choosing a region

The selection uses the OS's own selector where there is one. macOS: the system crosshair (`screencapture -i`). Linux: whichever of `grim`+`slurp`, `maim`, `scrot`, `spectacle`, `gnome-screenshot` or `flameshot` is installed (the Wayland ones first on Wayland). Otherwise, and on Windows, texcopy's own overlay: a full-screen window over your primary monitor showing a frozen, dimmed screenshot; drag to select, Esc to cancel. `TEXCOPY_SELECTOR=overlay` forces the overlay. `texcopy --select` uses the same selector without opening the window.

## The vision engine

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

How it runs: from the command line, each texcopy run starts `llama-server` on a free local port, reads the image, and stops it again, so nothing stays in memory between runs. On an Apple M4 Pro that is about 1 second end to end, with about 1.4 GB of RAM while it is running. The window instead keeps the helper loaded between captures (about 0.3 s per read) and unloads it after 2.5 minutes idle. The first run after installing can take longer while macOS checks the new helper. Everything stays on your machine.

Details: the vision engine does not report per-token confidence, so the "unsure about" warnings only exist for the small model. Its text gets the same math-marking rules as prose mode (lone letters like `v` become `$v$`).

## Small-model prose mode

With `--engine small`, the model is trained on formulas, so it writes sentences as `\mathrm{W o r d s ~ l i k e ~ t h i s}`, wraps visual lines in an `array`, and doesn't say which letters were italic. Prose mode (`src/prose.rs`) flattens the lines into one paragraph, glues words broken at a line end (`cut-` + `vertex`), re-joins a formula cut by a line break, and wraps the math in `$...$` using rules: a lone letter other than `a`, `A`, `I` is math; so are intervals like `[a,b]`, expressions with `= < > + ^ _`, and digit-letter mixes like `n-1` (but not `3-regular`). It only activates when there are at least four real words and no more math-like tokens than word-like ones, so formulas with a bit of `\text{if }` are untouched. `--wrap` is ignored for sentences, and `--json` reports `"kind": "prose"` or `"math"`.

Limits of the small model (measured with `--engine small` on `tools/bench.py --suite prose`, 21 cases, 17 exact; the vision engine reads 3-6 line paragraphs):

- **One or two lines work; three or more are unreliable.** The model reads the whole image at once and on a tall block it can lose the spaces between words (`beaconnectedgraphwith`). texcopy warns when it sees that. Select one line at a time.
- **It can't see italics**, so it cannot tell the variable `a` from the article "a" (both stay words), and it sometimes reads an italic `v` as `\nu`. texcopy prints a note whenever it outputs `\nu`.
- Wording errors by the model (`Euler` read as `Enler`) are flagged by the low-confidence warning when it was unsure.
