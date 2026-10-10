# Contributing to texcopy

Thanks for wanting to help. texcopy is small enough that one person can understand all of it, and bug reports with a screenshot are the most useful thing you can send.

## Reporting a bug

Open an [issue](https://github.com/Phoenixxo/texcopy/issues/new/choose) and include:

- **The picture you selected.** Attach the screenshot, cropped to the part you captured.
- **What texcopy gave you, and what you expected.** `texcopy image.png --json` prints the output, which engine read it, and the model's raw text. That raw text is the most useful thing for debugging.
- Your OS, your texcopy version (`texcopy --version`) and the engine (`auto`, `small` or `vlm`).

If the picture is copyrighted (a textbook page, say), crop it down to the smallest bit that shows the problem, or recreate it with your own text.

## Setting up

You need Rust 1.95 or newer ([rustup.rs](https://rustup.rs)). The README's [Building](README.md#-building) section has the Linux packages.

```bash
git clone https://github.com/Phoenixxo/texcopy.git
cd texcopy
cargo build --release
cargo test
```

Before you push, run the same checks CI does:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## The benchmark (please use it)

`tools/bench.py` renders known LaTeX, runs texcopy on the picture and grades the result (see [docs/BENCHMARKS.md](docs/BENCHMARKS.md)). It needs `pdflatex`, `pdftoppm` and ImageMagick.

If your change touches how text gets read or rebuilt (`src/vlm.rs`, `src/convert.rs`, `src/table.rs`, `src/prose.rs`, `src/tidy.rs`), **compare against the previous build.** Don't just look at the totals:

```bash
git stash && cargo build --release && python3 tools/bench.py --save before.json
git stash pop && cargo build --release && python3 tools/bench.py --baseline before.json
```

The diff lists every case that got better or worse, by name. Totals can hide a regression that a better case pays for. The diff can't. A bug fix should come with a test, and a new kind of input should come with a benchmark case.

## Where the code is

| Path | What it does |
|---|---|
| `src/gui.rs` | the window: the UI thread, a worker thread that owns the models, a capture thread |
| `src/engine.rs` | picking and running the engines, and what stays loaded between reads |
| `src/vlm.rs`, `src/helper.rs`, `src/fetch.rs` | the vision engine, its `llama-server` helper, verified downloads |
| `src/convert.rs`, `src/table.rs` | the model's Markdown / HTML into LaTeX |
| `src/prose.rs`, `src/tidy.rs`, `src/model.rs` | the small formula model and its clean-up |
| `src/capture.rs`, `src/overlay.rs` | picking a region on each OS |
| `docs/ARCHITECTURE.md` | a longer map of how the pieces fit, and why |

## Changing dependencies

Check that a new dependency has a permissive license (MIT, Apache-2.0, BSD, ISC, Zlib). Please avoid GPL, AGPL and LGPL-only crates, they'd change what we're able to ship.

You don't need to update any notices file. The license notices that MIT, Apache and BSD want to travel with a download get generated from `Cargo.lock` when the packages are built (`tools/third_party_licenses.py`), so they stay in step on their own.

## Style

- `cargo fmt`, and no clippy warnings. Match the code around you.
- Comments say *why*, not what. If a platform does something surprising, explain it right where it happens.
- No `unsafe` without a comment right above it saying why it's sound.
- Keep a change to one idea. A few small pull requests beat one big one.
- **Commit messages** are one short line in [Conventional Commits](https://www.conventionalcommits.org) style, like `fix: keep the blank line between paragraphs` or `feat: read tables from plain lines`. Types are `feat`, `fix`, `docs`, `test`, `refactor` and `chore`. No commit body.

## Pull requests

Fork, branch off `main`, open the pull request against `main`. CI (`CI` and `Tests` on macOS, Windows and Linux) has to pass. Fill in the template. If it's more than a small fix, open an issue first so we agree on the approach before you write it.

## What would help most right now

- **Try it on Windows and Linux** and tell me what breaks. I've only run it by hand on macOS.
- **Screenshots that fail.** Different fonts, journals and slide decks especially.
- **Packaging.** Homebrew, winget, Scoop and AUR recipes.
