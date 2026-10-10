# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's private reporting instead: [Report a vulnerability](https://github.com/Phoenixxo/texcopy/security/advisories/new). Include what you found, how to reproduce it, and which version.

You will get a reply within a few days. Fixes ship in a new release, and the advisory credits you unless you prefer otherwise.

## Supported versions

Only the latest release is supported.

## What texcopy does that matters for security

- **It runs locally.** Screenshots and recognised text are never sent anywhere.
- **Its only network requests** are downloads of the models (Hugging Face) and the `llama-server` helper (GitHub releases). Each download's size and SHA-256 are pinned in the source (`src/vlm.rs`, `src/helper.rs`) and checked before use; a mismatch deletes the file.
- **The vision helper** is `llama-server` started on `127.0.0.1` on a random free port, for the lifetime of the app, and stopped when texcopy exits (or by the next texcopy run if it crashed).
- **It reads your screen** only when you press Capture, and only the region you select. On macOS this needs the Screen Recording permission.
- **Release builds** are produced by the GitHub Actions workflow in `.github/workflows/release.yml` and published with SHA-256 checksums. The macOS app is ad-hoc signed (not notarised) and the Windows build is unsigned.
