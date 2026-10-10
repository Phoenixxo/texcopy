# Contributing to texcopy

Thanks for wanting to help. texcopy is small enough that one person can understand all of it, and bug reports with a screenshot are the most useful contribution of all.

## Reporting a bug

Open an [issue](https://github.com/Phoenixxo/texcopy/issues/new/choose) and include:

- **The picture you selected** (a screenshot you can attach; crop it to the part you captured).
- **What texcopy gave you** and what you expected. `texcopy image.png --json` prints the output, which engine read it, and the model's raw text, which is the most useful thing for debugging.
- Your OS, texcopy version (`texcopy --version`) and engine (`auto`, `small` or `vlm`).

If the picture is copyrighted (a textbook page), please crop it to the smallest region that shows the problem, or recreate it with your own text.

## Setting up

You need Rust 1.95 or newer ([rustup.rs](https://rustup.rs)); see the README's [Building](README.md#-building) section for the Linux packages.

```bash
git clone https://github.com/Phoenixxo/texcopy.git
cd texcopy
cargo build --release
cargo test
```

Before you push, the same checks CI runs:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## The benchmark (please use it)

`tools/bench.py` renders known LaTeX, runs texcopy on the picture and grades the result (see [docs/BENCHMARKS.md](docs/BENCHMARKS.md)). It needs `pdflatex`, `pdftoppm` and ImageMagick.

If your change touches how text is read or rebuilt (`src/vlm.rs`, `src/convert.rs`, `src/table.rs`, `src/prose.rs`, `src/tidy.rs`), **compare against the previous build**, do not just look at the totals:

```bash
git stash && cargo build --release && python3 tools/bench.py --save before.json
git stash pop && cargo build --release && python3 tools/bench.py --baseline before.json
```

The diff lists every case that got better or worse by name. Aggregate numbers hide a regression that a better case pays for; the case-by-case diff does not. A bug fix should come with a test, and a new kind of input with a benchmark case.

## Where the code is

| Path | What it does |
|---|---|
| `src/gui.rs` | the window: UI thread, a worker thread that owns the models, a capture thread |
| `src/engine.rs` | choosing and running the engines; what stays loaded between reads |
| `src/vlm.rs`, `src/helper.rs`, `src/fetch.rs` | the vision engine, its `llama-server` helper, verified downloads |
| `src/convert.rs`, `src/table.rs` | the model's Markdown / HTML into LaTeX |
| `src/prose.rs`, `src/tidy.rs`, `src/model.rs` | the small formula model and its clean-up |
| `src/capture.rs`, `src/overlay.rs` | selecting a region on each OS |
| `docs/REVIEW.md` | a longer tour for maintainers, with a review checklist |

## Style

- `cargo fmt` and no clippy warnings. Match the code around you.
- Comments say *why*, not what. Explain surprising platform behaviour where it happens.
- No `unsafe` without a comment directly above it saying why it is sound.
- Keep a change to one idea. Several small pull requests beat one big one.
- **Commit messages** are one line in [Conventional Commits](https://www.conventionalcommits.org) style: `fix: keep the blank line between paragraphs`, `feat: read tables from plain lines`. Types: `feat`, `fix`, `docs`, `test`, `refactor`, `chore`.

## Changing dependencies

Please check that a new dependency's license is permissive (MIT, Apache-2.0, BSD, ISC, Zlib). Avoid GPL, AGPL and LGPL-only crates: they would change what we can ship.

You do not need to update any notices file. The license notices that MIT, Apache and BSD require to travel with a download are generated from `Cargo.lock` when the packages are built (`tools/third_party_licenses.py`), so they stay in step with the dependencies automatically.

## Pull requests

Fork, branch off `main`, and open the pull request against `main`. CI (`CI` and `Tests` on macOS, Windows and Linux) must pass. Fill in the template; for anything bigger than a small fix, open an issue first so we agree on the approach before you write it.

## What would help most right now

- **Try it on Windows and Linux** and report what breaks. Only macOS has been run by hand so far.
- **Screenshots that fail**, especially from different fonts, journals and slide decks.
- Packaging: Homebrew, winget, Scoop and AUR recipes.
