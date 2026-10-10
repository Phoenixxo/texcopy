# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0]

First release.

### Added
- Read an image of LaTeX-rendered content and put its LaTeX on the clipboard: single formulas, sentences with inline math, numbered and unnumbered display equations, lists, headings and tables.
- A small window (Capture button, `wrap` and `engine` options, the output text), and a `texcopy` command line with JSON output.
- Two engines: a 118 MB formula model that runs in-process, and an optional vision model (OvisOCR2 through a local `llama-server`) with a guided, checksum-verified download.
- The window pre-loads the vision engine while you drag out the region and unloads it when idle.
- Region selection through the OS's own tool (macOS `screencapture`, common Linux screenshot tools) or texcopy's built-in overlay.
- Packages for macOS (`.app`), Windows (zip with a console-free launcher) and Linux (tarball with installer), built by a release workflow.
- A benchmark that re-renders the output and compares it with the original (`tools/bench.py`).

### Known limitations
- Developed and run by hand on macOS (Apple Silicon) only; the Windows and Linux builds are built and tested by CI but not yet exercised on real machines.
- Styling that is not part of the content (bold, italic, vertical table rules) is not recovered.
