//! The built-in region selector, used where the OS has none of its own (Windows, and Linux without
//! `grim`/`maim`/`spectacle`/...). It runs as its own short-lived process: `texcopy --select-overlay
//! <out.png>`. The window launches it the way it launches macOS's `screencapture`.
//!
//! The process takes a screenshot of the primary monitor *before* it creates any window (so the overlay
//! can never appear in its own picture), shows that frozen picture dimmed over the whole screen, lets
//! you drag a rectangle, and writes the cropped PNG. Esc, or a closed window, writes nothing.
//!
//! Being its own process matters: the overlay is then always the first window the process makes, which
//! is what gets it exactly the frame it asks for (a second window may be moved by the window manager).

use std::path::PathBuf;

use anyhow::{Context, Result};
use eframe::egui::{
    self, Color32, CursorIcon, Key, Pos2, Rect, ViewportBuilder, ViewportCommand, pos2, vec2,
};
use image::RgbaImage;

/// A drag smaller than this (in screenshot pixels) is a click, not a selection.
const MIN_SIDE_PX: u32 = 6;

/// Where a selection (in egui points inside the overlay window) lands in the screenshot, as
/// `(x, y, w, h)` in pixels. The scale is worked out from the window's real size, so it is right
/// whatever DPI the OS reports. None when the drag is too small to be a selection.
fn crop_rect(sel: Rect, window: Rect, shot: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let (sx, sy) = (
        shot.0 as f32 / window.width(),
        shot.1 as f32 / window.height(),
    );
    let clamp = |v: f32, max: u32| v.clamp(0.0, max as f32).round() as u32;
    let x0 = clamp((sel.min.x - window.min.x) * sx, shot.0);
    let y0 = clamp((sel.min.y - window.min.y) * sy, shot.1);
    let x1 = clamp((sel.max.x - window.min.x) * sx, shot.0);
    let y1 = clamp((sel.max.y - window.min.y) * sy, shot.1);
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    (w >= MIN_SIDE_PX && h >= MIN_SIDE_PX).then_some((x0, y0, w, h))
}

struct Overlay {
    shot: RgbaImage,
    texture: egui::TextureHandle,
    out: PathBuf,
    press: Option<Pos2>,
    now: Option<Pos2>,
    started: std::time::Instant,
    /// Test hook: `TEXCOPY_OVERLAY_AUTO=x0,y0,x1,y1` (points) "drags" this rectangle after 2 s.
    auto: Option<Rect>,
}

impl Overlay {
    fn selection(&self) -> Option<Rect> {
        Some(Rect::from_two_pos(self.press?, self.now?))
    }

    fn finish(&mut self, ctx: &egui::Context, sel: Rect) {
        let window = ctx.content_rect();
        let Some((x, y, w, h)) = crop_rect(sel, window, (self.shot.width(), self.shot.height()))
        else {
            // a click or a tiny drag: keep waiting for a real selection
            self.press = None;
            self.now = None;
            return;
        };
        let crop = image::imageops::crop_imm(&self.shot, x, y, w, h).to_image();
        if let Err(e) = crop.save(&self.out) {
            eprintln!("could not save the selection: {e}");
            std::process::exit(1); // the parent reports this as an error, not as a cancel
        }
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

impl eframe::App for Overlay {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(Key::Escape) || i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        ctx.request_repaint(); // follow the mouse
        if let Some(sel) = self.auto
            && self.started.elapsed() > std::time::Duration::from_secs(2)
        {
            self.auto = None;
            self.finish(ctx, sel);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.set_cursor_icon(CursorIcon::Crosshair);
        let (pressed, down, released, hover) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.hover_pos(),
            )
        });
        if pressed {
            self.press = hover;
        }
        if down || released {
            self.now = hover.or(self.now);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let full = ui.max_rect();
                let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                let p = ui.painter();
                p.image(self.texture.id(), full, whole, Color32::WHITE);
                p.rect_filled(full, 0.0, Color32::from_black_alpha(110));
                if let Some(sel) = self.selection() {
                    // the selected part shows undimmed, as a hole in the dimming
                    let uv = Rect::from_min_max(
                        pos2(
                            (sel.min.x - full.min.x) / full.width(),
                            (sel.min.y - full.min.y) / full.height(),
                        ),
                        pos2(
                            (sel.max.x - full.min.x) / full.width(),
                            (sel.max.y - full.min.y) / full.height(),
                        ),
                    );
                    p.image(self.texture.id(), sel, uv, Color32::WHITE);
                    p.rect_stroke(
                        sel,
                        0.0,
                        egui::Stroke::new(1.5, Color32::from_rgb(80, 160, 255)),
                        egui::StrokeKind::Outside,
                    );
                }
            });
        if released && let Some(sel) = self.selection() {
            self.finish(&ctx, sel);
        }
    }
}

/// Run the selector: capture the primary monitor, let the user drag, write the crop to `out`.
/// Returns Ok with no file written when the user cancelled.
pub fn run(out: PathBuf) -> Result<()> {
    std::fs::remove_file(&out).ok(); // so "no file afterwards" always means "cancelled"

    // Capture BEFORE any window exists, so the overlay is never in its own screenshot.
    let monitors = xcap::Monitor::all().context("listing monitors")?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or(monitors.first())
        .context("no monitor found")?;
    let shot = monitor.capture_image().context("capturing the screen")?;
    let (mx, my) = (monitor.x().unwrap_or(0), monitor.y().unwrap_or(0));
    let (mw, mh) = (monitor.width().unwrap_or(0), monitor.height().unwrap_or(0));
    anyhow::ensure!(
        shot.width() > 0 && shot.height() > 0,
        "the screenshot is empty"
    );

    let mut viewport = ViewportBuilder::default()
        .with_title("texcopy")
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false);
    if cfg!(target_os = "macos") {
        // Only reachable on macOS when testing (TEXCOPY_FORCE_OVERLAY): cover the monitor with a plain
        // borderless window, because native fullscreen would switch Spaces.
        viewport = viewport
            .with_position(pos2(mx as f32, my as f32))
            .with_inner_size(vec2(mw as f32, mh as f32));
    } else {
        // Fullscreen covers the monitor in whatever units the OS uses, and works on Wayland, where a
        // window cannot position itself.
        viewport = viewport.with_fullscreen(true);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "texcopy",
        options,
        Box::new(move |cc| {
            let img = egui::ColorImage::from_rgba_unmultiplied(
                [shot.width() as usize, shot.height() as usize],
                shot.as_raw(),
            );
            let texture = cc
                .egui_ctx
                .load_texture("screen", img, egui::TextureOptions::LINEAR);
            Ok(Box::new(Overlay {
                shot,
                texture,
                out,
                press: None,
                now: None,
                started: std::time::Instant::now(),
                auto: std::env::var("TEXCOPY_OVERLAY_AUTO").ok().and_then(|v| {
                    let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    (n.len() == 4).then(|| Rect::from_min_max(pos2(n[0], n[1]), pos2(n[2], n[3])))
                }),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    #[test]
    fn a_selection_maps_to_screenshot_pixels_at_any_scale() {
        // 1512x982 points shown for a 3024x1964-pixel screenshot (a 2x Retina display)
        let window = rect(0.0, 0.0, 1512.0, 982.0);
        assert_eq!(
            crop_rect(rect(200.0, 150.0, 720.0, 340.0), window, (3024, 1964)),
            Some((400, 300, 1040, 380))
        );
        // 1x display, and a window that is not at the origin
        let window = rect(10.0, 20.0, 1930.0, 1100.0);
        assert_eq!(
            crop_rect(rect(110.0, 120.0, 310.0, 220.0), window, (1920, 1080)),
            Some((100, 100, 200, 100))
        );
        // a screenshot of a different size than the window (fractional scaling): ratio comes from sizes
        let window = rect(0.0, 0.0, 1280.0, 720.0);
        assert_eq!(
            crop_rect(rect(640.0, 360.0, 1280.0, 720.0), window, (1920, 1080)),
            Some((960, 540, 960, 540))
        );
    }

    #[test]
    fn a_drag_past_the_edge_is_clamped_and_a_click_is_not_a_selection() {
        let window = rect(0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            crop_rect(rect(80.0, 80.0, 500.0, 500.0), window, (100, 100)),
            Some((80, 80, 20, 20))
        );
        assert_eq!(
            crop_rect(rect(10.0, 10.0, 12.0, 12.0), window, (100, 100)),
            None
        );
        assert_eq!(
            crop_rect(rect(10.0, 10.0, 11.0, 90.0), window, (100, 100)),
            None
        );
    }
}
