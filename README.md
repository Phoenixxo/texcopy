# texcopy

Drag-select anything on screen that was rendered with LaTeX and get its LaTeX source on your clipboard.

```
$ texcopy                      # crosshair appears; drag over a formula
\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}
Copied to clipboard.
```

Everything runs locally. Nothing is uploaded.

## How it works

LaTeX output (a screenshot, a PDF page) doesn't contain its source, so recovering it is OCR:

1. The image comes from a file, the clipboard, a screenshot (`--screen`, cross-platform via `xcap`) or, on macOS, `screencapture -i` drag-select.
2. The image is flattened onto white, inverted if it's dark mode, and resized to the model's 384x384 input (stretched by default, which handles wide formulas best; `--fit pad` keeps the aspect ratio instead).
3. A vision transformer (the *encoder*) turns it into 578 feature vectors.
4. A small transformer *decoder* writes LaTeX tokens one at a time, looking at those vectors, until it emits end-of-sequence.
5. `tidy.rs` rewrites the token stream (`\frac { a } { b }`) into idiomatic LaTeX (`\frac{a}{b}`). It only changes formatting, never meaning.

The model is [`breezedeus/pix2text-mfr`](https://huggingface.co/breezedeus/pix2text-mfr) (MIT), run through ONNX Runtime via the [`ort`](https://crates.io/crates/ort) crate. About 118 MB, downloaded on first run to `~/Library/Caches/texcopy` (or `$XDG_CACHE_HOME/texcopy`). A `./models` directory or `--model-dir` / `$TEXCOPY_MODEL_DIR` takes precedence.

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
texcopy                     # drag-select a screen region (macOS; other OSes: see below)
texcopy formula.png         # from a file
texcopy --clipboard         # from an image on the clipboard
texcopy --screen --region 100,200,600,150   # capture the primary monitor, crop to x,y,w,h (screenshot pixels)
texcopy --screen --monitor 1                # a specific monitor
texcopy -w inline           # wrap in $...$   (-w display: \[ ... \])
texcopy --no-copy           # print only
texcopy --raw               # skip tidy, show the model's raw tokens
texcopy --fit pad           # pad to the model's aspect ratio instead of stretching
texcopy --json              # latex + stop reason + per-token confidence as JSON (no clipboard)
```

Targets: Windows x64, macOS on Apple Silicon, Linux x64 (X11; Wayland best-effort). Drag-to-select is built in on macOS only for now; a cross-platform selector window is planned. Until then, on Windows and Linux use `--screen --region`, a file, or `--clipboard` with your OS's snipping tool.

macOS asks for Screen Recording permission for your terminal the first time you capture the screen.

**Linux clipboard:** X11 and Wayland serve the clipboard from the program that set it, so texcopy leaves a small background helper running that keeps the text available until you copy something else.

## Benchmark

`tools/bench.py` renders known LaTeX with `pdflatex`, runs texcopy on the image, compiles the output and compares the two renders pixel by pixel. It needs `pdflatex`, `pdftoppm` and ImageMagick, and is a dev tool only.

```
python3 tools/bench.py                                   # all suites
python3 tools/bench.py --baseline tools/baseline.json    # report what got better or worse
```

## Limitations

- **Proofread the output.** texcopy warns about tokens the model was under 50% sure of, and refuses to copy output where the model got stuck repeating itself, but a confident answer can still be wrong. It is a small model. Typical mistakes: `\epsilon` read as `e`, `\bar` as `\vec`, `\text{...}` as `\mathrm{...}`.
- The cleaned-up output uses `\text`, `\binom` and `pmatrix`-style environments, so the document you paste into needs `amsmath` (use `--raw` to see exactly what the model produced).
- One formula per selection. Paragraphs, tables and multi-line alignments are out of scope.
- Interactive drag-select is macOS-only for now. Only macOS has been run so far; the Windows and Linux code paths (including the Linux clipboard helper) are written but not yet exercised.
- The model's vocabulary is only 1,200 tokens, so custom macros and rarely used symbols can't be represented.

If the source is a web page using KaTeX or MathJax, the exact TeX is already in the page's DOM and no OCR is needed.
