# Benchmarks

I don't want to just demo texcopy, I want to measure it. `tools/bench.py` renders known LaTeX with `pdflatex`, runs texcopy on the picture, and grades the answer by *re-rendering it and comparing to the original*. So an answer that's different but equivalent still counts as right.

## Latest results

Measured on an Apple M4 Pro with the vision engine (`--engine auto`), 182 cases:

| Suite | What it checks | Exact | Close | Differs | Failed | Cases |
|---|---|---:|---:|---:|---:|---:|
| `basic` (T1-T3) | single formulas, from `E=mc^2` to nested continued fractions | 15 | 0 | 0 | 0 | 15 |
| `basic` (T4) | multi-line: `aligned`, `cases`, matrices | 2 | 0 | 1 | 0 | 3 |
| `degraded` | the same formulas at 50-800 dpi, blurred, noisy, inverted, JPEG, other fonts | 28 | 2 | 0 | 0 | 30 |
| `prose` | sentences with inline math at two widths and two resolutions | 17 | 3 | 0 | 0 | 20 |
| `real` | 45 exercises cut from a real graph-theory textbook (kept private) | 44 | 1 | 0 | 0 | 45 |
| `tables` | 18 tables, graded on their *cells* | 16 | 0 | 2 | 0 | 18 |
| `documents` | 36 page elements: equations, lists, headings, quotes, specials, theorems... | 15 | 1 | 20 | 0 | 36 |
| `stress` | very wide equations, big matrices, rare symbols, text inside math | 9 | 4 | 2 | 0 | 15 |
| **Total** | | **146** | **11** | **25** | **0** | **182** |

How to read the table:

- **Exact** means the output re-renders pixel-identical (formulas, documents), or matches word for word (prose, real), or cell for cell (tables). **Close** compiles and says the same thing with a small difference, like a dropped full stop or a layout change. **Differs** compiles but is different. **Failed** doesn't compile.
- Most `documents` misses are styling that the picture doesn't carry (bold, italic, small caps, theorem boxes). That's out of scope on purpose. The goal is the content, and the content is there.
- The two `tables` misses are a `rowspan` whose text sits in the other row of the span, and `g/cm$^3$`, which the model reads as `g/cm3`.
- With just the small model, the 45 real textbook exercises get 22 exact. The vision engine gets 44.

## Running them

`tools/bench.py` renders known LaTeX with `pdflatex`, runs texcopy on the image, compiles the output and compares the two renders pixel by pixel. It needs `pdflatex`, `pdftoppm` and ImageMagick, and it's a dev tool only.

```
python3 tools/bench.py                                   # all suites
python3 tools/bench.py --baseline tools/baseline.json    # report what got better or worse
```

`tools/book_cases.py` cuts real exercise screenshots out of a textbook PDF that has a text layer (ebook conversions work well) and takes the ground truth from the PDF's own text and italics. Put them in the gitignored `tools/fixtures/private/` and run `python3 tools/bench.py --suite real`. The crops are copyrighted, so they never get committed. `tools/bakeoff.py` scores vision-language OCR models served by llama.cpp on the same cases (see `tools/bakeoff.py --help`).

```
python3 tools/bench.py --suite documents     # one suite: basic, degraded, stress, prose, real, documents, tables
python3 tools/bench.py --save now.json       # save results, then later:
python3 tools/bench.py --baseline now.json   # a case-by-case diff against the saved run
```

`tools/baseline.json` has my results for every suite except the private `real` one. A case or two can differ on other hardware. **Always compare against the previous build before you trust a change.** Run the suites with the old binary (`--bin path/to/old/texcopy --save old.json`), run them with the new one, and diff. Totals hide regressions. The diff tells you which case got worse.

## Generated cases

The suites above are cases I wrote by hand, and hand-written cases can overfit to the problems I'd just seen. So there's also a generator that makes cases with real tools. It's a small Rust program in `tools/gen`, and every case is a picture of some LaTeX plus the exact source it came from.

- **Random formulas** from a small grammar (fractions, sums, integrals, matrices, `cases`, `aligned`), drawn with `pdflatex` and `xelatex` in a few fonts (Computer Modern, Latin Modern, STIX Two).
- **Sentences and small documents.** Paragraphs with inline math, display equations, lists, tables and headings.
- **Web style.** KaTeX in headless Chrome, light and dark, with a few fonts, like a page on screen.
- **Real LaTeX.** Equations, lists and tables cut out of CC-BY arXiv papers, compiled with each paper's own macros. `fetch-arxiv` only keeps papers whose arXiv page shows CC-BY, CC-BY-SA or CC0.
- **Degradation.** About a quarter get smaller, blurrier, noisier or JPEG-squashed, like a real screenshot.

The same seed always makes the same cases, so a failure can be recreated and reported. Nothing generated gets committed. The generator is the recipe and the pictures are build output (they go in `tools/fixtures/generated/`, which git ignores).

```
cargo run --release --manifest-path tools/gen/Cargo.toml -- fetch-arxiv --count 12      # once, for the real-paper cases
python3 tools/bench.py --suite generated --seed 7 --count 200                           # makes the cases if needed, then grades
python3 tools/bench.py --suite generated --seed 7 --count 60 --kinds corpus             # just the real-paper cases
python3 tools/bench.py --suite generated --seed 7 --count 200 --worst 20                # show the 20 worst misses
```

It grades by re-rendering both the truth and texcopy's answer with `pdflatex` and comparing, so it doesn't matter which renderer made the picture texcopy saw. All the pictures are read through one `texcopy --batch` run, so the model only loads once. The report breaks results down by category, renderer and degradation, with a 95% confidence interval on each rate. With a few cases in a group the interval is wide, and that's the honest answer.

You need `pdflatex`, `xelatex` and `pdftoppm`. The web-style cases also need Node and Chrome (KaTeX installs itself into `tools/.cache`). The generator leaves Times math (`mathptmx`) out on purpose, because on a BasicTeX install `\leq`, `\neq` and `\in` need the `rsfs` font, which isn't there. STIX Two covers the Times-like look instead.

### Latest generated results

200 cases from seed 7 (Apple M4 Pro, vision engine on):

| Category | Cases | Exact | Exact or close | Notes |
|---|---:|---:|---:|---|
| formula, simple | 30 | 28 | 28 | |
| formula, medium | 43 | 37 | 39 | |
| formula, hard | 33 | 31 | 33 | |
| matrix | 16 | 16 | 16 | |
| `cases` | 8 | 6 | 7 | |
| `aligned` (multi-line) | 7 | 2 | 3 | the weak spot, see below |
| sentences | 15 | 14 | 15 | |
| documents (text, equations, lists, tables) | 29 | 9 | 24 | most misses are equivalent LaTeX that moves a line break |
| web style (KaTeX), formulas | 13 | 8 | 8 | |
| web style (KaTeX), sentences | 6 | 5 | 5 | |
| **All** | **200** | **156 (78%)** | **178 (89%)** | 95% interval for exact: 72-83% |

By how the picture was damaged: untouched 80% exact (116 of 145), JPEG 88%, shrunk 75%, blurred 64%, noisy 58%. The small groups have wide intervals, so don't read much into the order. Noise and blur do hurt, though.

"Close" for a document means the content is the same once harmless differences are ignored (`\left(10\right)` versus `(10)`, a table inside `center`) but the picture differs, usually because a line broke somewhere else. It's graded this way because those differences can't be seen in the picture, so there's nothing to fix.

What the misses have in common, found by this suite:

- **`\binom` comes back as a two-row matrix.** The model draws it as `\left(\begin{matrix}n\\k\end{matrix}\right)`, which prints as a bigger `pmatrix`.
- **Multi-line equations lose their alignment.** They come back as a one-column `array`, so the `=` signs don't line up. One came back with broken braces.
- **The index of a root gets lost.** `\sqrt[n]{a}` comes back as `\sqrt{a}` or as a stray `\, 3 \! \! \! \sqrt{a}`.
- **A formula on a web page is read as a heading.** The model writes `## w ⊂ Q`, and raw Unicode like `⊂` doesn't compile.
- **Plain misreads.** `n` as `m`, an italic `v` as `\upsilon`, `\text{ even}` dropped.

### Real arXiv papers

37 cases from 8 CC-BY papers: 31 equations, 5 tables and 1 list (`--kinds corpus`, seed 7). Only 9 are exact (24%, with a 95% interval of 13-40%), and this is the hardest set. Real papers use custom macros, `align*`, `split` and `\toprule`, and they use bold in tables. It's a small sample, so take the exact number loosely. The patterns are the useful part:

- **How the equation number comes back decides the result.** When the model writes it as `\tag*{(1)}` (12 equations), 8 are exact or close, because that renders just like a real `equation`. When it writes `\qquad\qquad(1)` (6) or puts `(1)` on its own line (3), none match: the number sits next to the formula and not in the margin. Turning those into a real `equation` is the clearest fix this found.
- **Tables from real papers are 0 of 5.** The cells are usually right, but `\toprule`, bold headers and `p{4cm}` columns don't survive, and one came back unbalanced.
- **Multi-line `align*`, `gathered` and `split`** come back as `aligned` or a one-column `array`, so the alignment shifts.

## Where it gets things wrong

Sometimes the output is just wrong. It might not compile, it might render as something else, or it might be shaped differently from what you were looking at. I'd rather you expect that than get surprised by it. Every suite above has misses in it, and the real arXiv papers are where it struggles most.

If you hit one, please tell me. [Open an issue](https://github.com/Phoenixxo/texcopy/issues/new/choose) with the picture and what you got (`texcopy image.png --json` has the details). A pull request is even better. Fixes are welcome, and so are new benchmark cases that show a miss. [CONTRIBUTING.md](../CONTRIBUTING.md) has the setup.

The kinds of wrong I know about:

- **Misread symbols.** texcopy warns about tokens the small model was under 50% sure of, and it refuses to copy output where the model got stuck repeating itself. A confident answer can still be wrong though. Typical mistakes are `\epsilon` read as `e`, `\bar` as `\vec`, and `\text{...}` as `\mathrm{...}`. The vision engine doesn't report confidence, so it can't warn.
- **Wrong structure.** Equation numbers, `\binom`, multi-line `align`, root indexes and real-paper tables, all covered above.
- **Things it leaves out on purpose.** Bold, italic, small caps, underline and typewriter text, vertical table rules, theorem boxes and footnote layout aren't reliably in what the model reads, so they come out plain. The goal is a correct reconstruction of the content, not your source.
- **Packages.** The output uses `\text`, `\binom`, `equation`, `align` and `pmatrix`-style environments, so the document you paste into needs `amsmath`. Tables don't need a package. (`--raw` shows exactly what the small model produced.)
- **Fonts I haven't tried.** Times, txfonts and Latin Modern math all read exactly in the benchmark. I haven't tested Palatino, Utopia, Charter and so on because the TeX install the benchmark runs on doesn't have them.
- **Windows and Linux.** I've only run texcopy by hand on macOS. CI builds and tests the other two, but I haven't tried the clipboard, capture, built-in selector overlay or helper install on real machines (or the vision engine's CPU speed there). The overlay only covers the primary monitor.
- **Rare symbols.** The small model only knows 1,200 tokens, so custom macros and rarely used symbols can't be represented.

If what you're copying from is a web page using KaTeX or MathJax, the exact TeX is already in the page's DOM and you don't need OCR at all.
