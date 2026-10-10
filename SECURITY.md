# Security policy

## Reporting a vulnerability

Please **don't open a public issue** for a security problem. Use GitHub's private reporting instead: [Report a vulnerability](https://github.com/Phoenixxo/texcopy/security/advisories/new). Tell me what you found, how to reproduce it, and which version.

I'll reply within a few days. Fixes go out in a new release, and the advisory credits you unless you'd rather it didn't.

## Supported versions

Just the latest release.

## What texcopy does that matters for security

- **It runs locally.** Your screenshots and the recognised text never get sent anywhere.
- **The only network requests** are downloads of the models (Hugging Face) and the `llama-server` helper (GitHub releases). Each download's size and SHA-256 are pinned in the source (`src/vlm.rs`, `src/helper.rs`) and checked before use. A mismatch deletes the file.
- **The vision helper** is `llama-server` started on `127.0.0.1` on a random free port. It runs while the app does and gets stopped when texcopy exits (or by the next texcopy run if it crashed).
- **It reads your screen** only when you press Capture, and only the region you pick. On macOS that needs the Screen Recording permission.
- **Release builds** come from the GitHub Actions workflow in `.github/workflows/release.yml` and are published with SHA-256 checksums. The macOS app is ad-hoc signed (not notarised) and the Windows build is unsigned.
