# Phase 2 spike: findings

Question: can one egui app go from a frozen full-screen overlay (drag to select) to a small popup, reliably?
Tested on an Apple M4 Pro, 1512x982 pt Retina display (3024x1964 px), eframe/egui 0.36, with AeroSpace (a tiling window manager) running.

## Verdict
Yes, with these constraints. The self-test (`cargo run --release -- --selftest`, synthetic test pattern, no real capture) passed 3 of 3 runs, 5 checks each.

| Check | Result |
|---|---|
| Overlay covers the monitor exactly, no offset | pass, `inner_rect` (0,0)-(1512,982) |
| Retina crop: drag of (200,150)-(720,340) pt | pass, pixels x=400 y=300 1040x380, first pixel matches |
| Popup keeps its requested size and position | pass, 520x230 centred, unchanged after 1.5 s |
| Launch to overlay visible | 120-160 ms |
| Selection to popup visible | about 40 ms (the engine then takes about 1 s) |
| No process left running | pass |

## What went wrong first (and the rules that came out of it)
1. **Do not turn the overlay into the popup.** Toggling a borderless window to decorated made macOS re-apply its own frame a few frames later (sizes like 744x905 or 1466x905, a different one each run).
2. **The cause of the "random" frames was AeroSpace**, a tiling window manager that was running: every new, normal-looking window was tiled (744x905 at (760,38) is its right-half tile). Any user with a tiling manager will see this.
3. **Popup: no minimize/zoom buttons.** AeroSpace treats a window without them as a dialog and floats it (resizable can stay on). Verified: with both buttons present it was tiled; with either off it stayed put.
4. **Overlay: make it the root (first) window.** As a second window it was pushed down 64 pt (`covers_monitor=false`) and the crop was off by 64 pt; as the first window it gets exactly the frame asked for.
5. After the selection, the root window is shrunk to 1x1 and made transparent (it stays borderless), and the popup is a second window created fresh.
6. Capture the screen **before** creating any window, so the overlay never appears in its own screenshot.
7. A selftest that synthesises the selection from the wrong window computed the wrong origin (it used the root's `inner_rect`). Selection and origin must come from the same window.

## Not verified here
- Real screen capture and real mouse dragging (the self-test uses a synthetic pattern and a synthetic selection). Needs Screen Recording permission for the terminal.
- Windows and Linux, multi-monitor, and a Mac without a tiling manager.
- The menu bar: the overlay reports covering y=0, but whether the menu bar still draws above it was not checked visually.

## Round 2: window first, then Capture (user feedback: "the gui should pop up first then I can click to screenshot")
The custom overlay went straight to a full-screen capture, so there was no window to see or click. New flow:
1. A small window opens (options, output text, a Capture button; `C` also works).
2. Capture hides the window, waits 250 ms, and runs the system selector (`screencapture -i`) in a thread.
3. The window comes back with the result; texcopy runs on the saved PNG and the text is copied to the clipboard.

Why the system selector instead of our overlay on macOS: it avoids every problem in round 1 (tiling managers, the 64 pt
offset of a second window, the menu bar, multiple monitors, Retina maths), and it is the crosshair people already know.
`cargo run --release -- --selftest` (with `TEXCOPY_FAKE_CAPTURE=<png>`) checks the hide, capture, show, engine loop
without a mouse: the update loop keeps running while the window is hidden, and textbook 5.4 comes back exact.
Windows and Linux still need their own selector (the round 1 overlay is in git history, commit 3671b4a).
