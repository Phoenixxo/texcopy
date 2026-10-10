# Benchmarks and limitations

texcopy is measured, not just demoed. `tools/bench.py` renders known LaTeX with `pdflatex`, runs texcopy on the picture, and grades the result by *re-rendering the output and comparing it to the original*, so a different-but-equivalent answer still counts.

## Latest results

Measured on an Apple M4 Pro with the vision engine (`--engine auto`), 183 cases:

| Suite | What it checks | Exact | Close | Differs | Failed | Cases |
|---|---|---:|---:|---:|---:|---:|
| `basic` (T1-T3) | single formulas, from `E=mc^2` to nested continued fractions | 15 | 0 | 0 | 0 | 15 |
| `basic` (T4) | multi-line: `aligned`, `cases`, matrices | 2 | 0 | 1 | 0 | 3 |
| `degraded` | the same formulas at 50-800 dpi, blurred, noisy, inverted, JPEG, other fonts | 28 | 2 | 0 | 0 | 30 |
| `prose` | sentences with inline math at two widths and two resolutions | 18 | 3 | 0 | 0 | 21 |
| `real` | 45 exercises cut from a real graph-theory textbook (kept private) | 44 | 1 | 0 | 0 | 45 |
| `tables` | 18 tables, graded on their *cells* | 16 | 0 | 2 | 0 | 18 |
| `documents` | 36 page elements: equations, lists, headings, quotes, specials, theorems... | 15 | 1 | 20 | 0 | 36 |
| `stress` | very wide equations, big matrices, rare symbols, text inside math | 9 | 4 | 2 | 0 | 15 |
| **Total** | | **147** | **11** | **25** | **0** | **183** |

How to read it:

- **Exact** means the output re-renders pixel-identically (formulas, documents) or matches word for word (prose, real) or cell for cell (tables). **Close** compiles and says the same thing with a small difference (a dropped full stop, a layout change). **Differs** compiles but is different. **Failed** does not compile.
- Most `documents` misses are styling the picture does not carry (bold, italic, small caps, theorem boxes). That is out of scope by design: the goal is the content, and the content is there.
- The two `tables` misses are a `rowspan` whose text sits in the other row of the span, and `g/cm$^3$` that the model reads as `g/cm3`.
- Against the small model alone, the 45 real textbook exercises score 22 exact; the vision engine scores 44.

## Running them

`tools/bench.py` renders known LaTeX with `pdflatex`, runs texcopy on the image, compiles the output and compares the two renders pixel by pixel. It needs `pdflatex`, `pdftoppm` and ImageMagick, and is a dev tool only.

```
python3 tools/bench.py                                   # all suites
python3 tools/bench.py --baseline tools/baseline.json    # report what got better or worse
```

`tools/book_cases.py` cuts real exercise screenshots out of a textbook PDF that has a text layer (ebook conversions work well) and writes ground truth from the PDF's own text and italics. Put them in the gitignored `tools/fixtures/private/` and run `python3 tools/bench.py --suite real`; the crops are copyrighted, so they are never committed. `tools/bakeoff.py` scores vision-language OCR models served by llama.cpp on the same cases (see `tools/bakeoff.py --help`).

```
python3 tools/bench.py --suite documents     # one suite: basic, degraded, stress, prose, real, documents, tables
python3 tools/bench.py --save now.json       # save results, then later:
python3 tools/bench.py --baseline now.json   # a case-by-case diff against the saved run
```

`tools/baseline.json` holds the maintainer's results for every suite except the private `real` one; a case or two can differ on other hardware. **Always compare against the previous build before trusting a change.** Run the suites with the old binary (`--bin path/to/old/texcopy --save old.json`) and the new one, and diff. Aggregate numbers hide regressions; the diff names the case that got worse.

## Limitations


- **Proofread the output.** texcopy warns about tokens the small model was under 50% sure of, and refuses to copy output where the model got stuck repeating itself, but a confident answer can still be wrong. Typical small-model mistakes: `\epsilon` read as `e`, `\bar` as `\vec`, `\text{...}` as `\mathrm{...}`. The vision engine does not report confidence, so it cannot warn.
- The output uses `\text`, `\binom`, `equation`, `align` and `pmatrix`-style environments, so the document you paste into needs `amsmath`. Tables need no package. (`--raw` shows exactly what the small model produced.)
- **The output is a reconstruction.** Bold, italic, small caps, underline and typewriter text, vertical table rules, theorem boxes and footnote layout are not reliably in what the model reads, so they come out plain. That is by design, not a bug: the goal is the content.
- **Fonts:** Times, txfonts and Latin Modern math all read exactly in the benchmark. Other math fonts (Palatino, Utopia, Charter...) are untested, because the TeX installation the benchmark runs on doesn't ship them.
- **Only macOS has actually been run.** CI builds and tests Windows and Linux, but the Windows and Linux clipboard, capture, built-in selector overlay and helper install (and the vision engine's CPU speed there) have not been exercised on real machines. The overlay covers only the primary monitor.
- The small model's vocabulary is only 1,200 tokens, so custom macros and rarely used symbols can't be represented.

If the source is a web page using KaTeX or MathJax, the exact TeX is already in the page's DOM and no OCR is needed.
