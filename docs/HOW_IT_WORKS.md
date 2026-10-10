# How texcopy works

The short version is in the [README](../README.md#-how-it-works). This is the long one, for when you want to know what's going on under the hood or change something.

## The small model's pipeline

A screenshot of LaTeX doesn't contain the LaTeX, so getting it back is OCR. This is the path through the small formula model. The vision engine (further down) skips steps 2 to 4 and makes one call to a bigger model instead.

1. The image comes from a file, the clipboard, a screenshot (`--screen`, works everywhere through `xcap`) or a region you dragged (see *Choosing a region* below).
2. The image gets flattened onto white, inverted if it's dark mode, and resized to the model's 384x384 input. It's stretched by default because that handles wide formulas best. `--fit pad` keeps the aspect ratio instead.
3. A vision transformer (the *encoder*) turns it into 578 feature vectors.
4. A small transformer *decoder* writes LaTeX tokens one at a time, looking at those vectors, until it says it's done.
5. `tidy.rs` rewrites the token stream (`\frac { a } { b }`) into normal LaTeX (`\frac{a}{b}`). It only changes formatting, never meaning. If the image turns out to be a sentence and not a formula, `prose.rs` takes over (see *Small-model prose mode* below).

**Two engines.** The *small* one is described first. The *vision* one (OvisOCR2, a 0.8B vision-language model) reads whole paragraphs and very long equations, which the small one can't. See *The vision engine* below. `--engine auto` (the default) uses the vision engine if it's set up, and for a bare formula it still prefers the small model.

The small model is [`breezedeus/pix2text-mfr`](https://huggingface.co/breezedeus/pix2text-mfr) (MIT), run through ONNX Runtime with the [`ort`](https://crates.io/crates/ort) crate. It's about 118 MB and gets downloaded the first time you run it, into your OS's per-user cache folder (`~/Library/Caches/texcopy` on macOS, for example). A `./models` folder, `--model-dir` or `$TEXCOPY_MODEL_DIR` takes priority.

## Choosing a region

texcopy uses the OS's own selector when there is one. On macOS that's the system crosshair (`screencapture -i`). On Linux it's whichever of `grim` + `slurp`, `maim`, `scrot`, `spectacle`, `gnome-screenshot` or `flameshot` you have installed (the Wayland ones first on Wayland). If there isn't one, and on Windows, texcopy uses its own overlay: a full-screen window over your primary monitor with a frozen, dimmed screenshot. Drag to select, Esc to cancel. `TEXCOPY_SELECTOR=overlay` forces the overlay, and `texcopy --select` uses the same selector without opening the window.

## The vision engine

On 45 real exercises from a graph-theory textbook the small model gets 22 exactly right and the vision engine gets 44 (the one other differs only by a dropped full stop). It also reads 3 to 6 line paragraphs and equations with 20+ terms that the small model garbles.

```
texcopy --setup             # one-time: asks first, then downloads, with progress
texcopy problem.png         # --engine auto uses it from now on
texcopy --engine vlm x.png  # force it (sets itself up if needed)
```

`--setup` tells you what it's going to fetch and asks once. `--yes` answers for you, and without a terminal it refuses instead of guessing. It fetches two things:

1. **llama.cpp `llama-server`** (MIT), release `b11146`, 11 to 19 MB, from github.com/ggml-org/llama.cpp. The release and the SHA-256 of each platform's file are pinned in `src/helper.rs`, and a download that doesn't match gets deleted. Apple Silicon Macs get the Metal (GPU) build. Windows and Linux get the CPU build. If you want a GPU build there, install one yourself and pass `--llama-server`. If you already have a `llama-server` on your PATH and texcopy hasn't installed its own, it uses that.
2. **OvisOCR2** (Apache-2.0), 1.02 GB (an 8-bit model file plus its vision part) from huggingface.co/bartowski/ATH-MaaS_OvisOCR2-GGUF, checksum-verified.

Both go in texcopy's cache folder (`TEXCOPY_CACHE_DIR` changes it). Downloads show a progress bar with speed and time left, and an interrupted one picks up where it stopped.

How it runs: from the command line, every texcopy run starts `llama-server` on a free local port, reads the image and stops it again, so nothing stays in memory between runs. On an Apple M4 Pro that's about 1 second end to end, with about 1.4 GB of RAM while it's running. The window works differently. It keeps the helper loaded between captures (about 0.3 s per read) and unloads it after 2.5 minutes idle. The first run after installing can take longer while macOS checks the new helper. Everything stays on your machine.

One more thing: the vision engine doesn't report per-token confidence, so the "unsure about" warnings only exist for the small model. Its text gets the same math-marking rules as prose mode (a lone letter like `v` becomes `$v$`).

## Small-model prose mode

With `--engine small`, the model is trained on formulas, so it writes sentences like `\mathrm{W o r d s ~ l i k e ~ t h i s}`, wraps visual lines in an `array`, and doesn't say which letters were italic. Prose mode (`src/prose.rs`) flattens the lines into one paragraph, glues together words that were broken at the end of a line (`cut-` + `vertex`), re-joins a formula that got cut by a line break, and wraps the math in `$...$` using a few rules:

- a lone letter other than `a`, `A` or `I` is math
- so are intervals like `[a,b]`, anything with `= < > + ^ _`, and digit-letter mixes like `n-1` (but not `3-regular`)

It only kicks in when there are at least four real words and no more math-looking tokens than word-looking ones, so a formula with a bit of `\text{if }` in it is left alone. `--wrap` is ignored for sentences, and `--json` reports `"kind": "prose"` or `"math"`.

Limits of the small model (measured with `--engine small` on `tools/bench.py --suite prose`, 21 cases, 17 exact; the vision engine reads 3 to 6 line paragraphs):

- **One or two lines work, three or more are unreliable.** The model reads the whole image at once and on a tall block it can lose the spaces between words (`beaconnectedgraphwith`). texcopy warns when it sees that. Select one line at a time.
- **It can't see italics**, so it can't tell the variable `a` from the word "a" (both stay words), and it sometimes reads an italic `v` as `\nu`. texcopy prints a note whenever it outputs `\nu`.
- When the model gets a word wrong (`Euler` read as `Enler`), the low-confidence warning flags it if the model was unsure.
