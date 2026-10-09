//! Phase 2 spike: frozen-screenshot overlay -> drag-select -> minimal popup (options + output text).
//! See FINDINGS.md next to this file for what it taught us.
//!
//! Run from this folder:  cargo run --release     (needs ../../target/release/texcopy built)
//!
//!   gui-spike              capture the screen, drag-select, popup shows options + output text
//!   gui-spike --selftest   synthetic test-pattern screen, synthetic selection, prints PASS/FAIL
//!   gui-spike --no-engine  don't run texcopy (popup shows a placeholder)
//!
//! Why two windows: turning the borderless overlay into a decorated popup made macOS keep
//! re-applying its own frame (see the findings). A window created with the right attributes
//! from the start behaves. A 1x1 transparent root window only drives the frame loop.

use std::{
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Color32, CursorIcon, Key, Pos2, Rect, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId, pos2, vec2,
};
use image::RgbaImage;

struct Mon {
    x: i32,
    y: i32,
    w: u32, // points
    h: u32,
}

#[derive(PartialEq, Clone, Copy)]
enum Phase {
    Select,
    Popup,
}

#[derive(PartialEq, Clone, Copy)]
enum Wrap {
    None,
    Inline,
    Display,
}

struct Reading {
    latex: String,
    kind: String,
}

struct App {
    t0: Instant,
    shot: RgbaImage,
    mon: Mon,
    scale: f32,
    tex: egui::TextureHandle,
    phase: Phase,
    press: Option<Pos2>,
    now: Option<Pos2>,
    crop_png: Option<std::path::PathBuf>,
    rx: Option<mpsc::Receiver<Result<Reading, String>>>,
    reading: Option<Reading>,
    error: Option<String>,
    wrap: Wrap,
    engine: String,
    last_copied: String,
    selftest: bool,
    use_engine: bool,
    frame: u32,
    overlay_logged: bool,
    popup_since: Option<Instant>,
    popup_frames: u32,
    checks: Vec<(String, bool)>,
    quit: bool,
    expected: Option<(u32, u32, u32, u32)>,
}

const POPUP_SIZE: (f32, f32) = (520.0, 230.0);

impl App {
    fn origin(ctx: &egui::Context, mon: &Mon) -> Pos2 {
        match ctx.input(|i| i.viewport().inner_rect) {
            Some(r) => pos2(mon.x as f32 - r.min.x, mon.y as f32 - r.min.y),
            None => Pos2::ZERO,
        }
    }

    fn selection(&self) -> Option<Rect> {
        Some(Rect::from_two_pos(self.press?, self.now?))
    }

    fn display_text(&self) -> String {
        let Some(r) = &self.reading else { return String::new() };
        // a sentence carries its own $...$ marks; wrapping only applies to a formula
        match (self.wrap, r.kind.as_str()) {
            (Wrap::None, _) | (_, "prose") => r.latex.clone(),
            (Wrap::Inline, _) => format!("${}$", r.latex),
            (Wrap::Display, _) => format!("\\[ {} \\]", r.latex),
        }
    }

    fn run_engine(&mut self) {
        let Some(path) = self.crop_png.clone() else { return };
        self.reading = None;
        self.error = None;
        if !self.use_engine {
            self.reading = Some(Reading { latex: "x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}".into(), kind: "math".into() });
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let engine = self.engine.clone();
        let bin = std::env::var("TEXCOPY_BIN").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/release/texcopy").into());
        std::thread::spawn(move || {
            let t = Instant::now();
            let out = Command::new(&bin).arg(&path).args(["--json", "--no-copy", "--engine", &engine]).output();
            let msg = match out {
                Ok(o) if o.status.success() => {
                    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
                    eprintln!("[engine] {engine}: {:.2}s", t.elapsed().as_secs_f32());
                    Ok(Reading {
                        latex: v["latex"].as_str().unwrap_or("").to_string(),
                        kind: v["kind"].as_str().unwrap_or("math").to_string(),
                    })
                }
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(format!("could not run {bin}: {e}")),
            };
            let _ = tx.send(msg);
        });
    }

    fn finish_selection(&mut self, ctx: &egui::Context, sel: Rect) {
        let o = Self::origin(ctx, &self.mon);
        let px = |p: Pos2| ((p.x - o.x) * self.scale, (p.y - o.y) * self.scale);
        let (x0, y0) = px(sel.min);
        let (x1, y1) = px(sel.max);
        let (iw, ih) = (self.shot.width() as f32, self.shot.height() as f32);
        let (x0, y0) = (x0.clamp(0.0, iw).round() as u32, y0.clamp(0.0, ih).round() as u32);
        let (x1, y1) = (x1.clamp(0.0, iw).round() as u32, y1.clamp(0.0, ih).round() as u32);
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        eprintln!("[select] points {:?} -> pixels x={x0} y={y0} w={w} h={h}", sel);
        if w < 6 || h < 6 {
            self.press = None;
            self.now = None;
            return;
        }
        let crop = image::imageops::crop_imm(&self.shot, x0, y0, w, h).to_image();
        self.expected = Some((x0, y0, w, h));
        if self.selftest {
            self.checks.push(("crop's first pixel is the pattern pixel under the selection".into(), *crop.get_pixel(0, 0) == pattern_pixel(x0, y0)));
        }
        let path = std::env::temp_dir().join(format!("gui-spike-{}.png", std::process::id()));
        crop.save(&path).expect("save crop");
        self.crop_png = Some(path);
        self.phase = Phase::Popup;
        self.popup_since = Some(Instant::now());
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1.0, 1.0)));
        ctx.send_viewport_cmd(ViewportCommand::Transparent(true));
        eprintln!("[timing] selection done {:.0} ms after process start", self.t0.elapsed().as_secs_f64() * 1000.0);
        if self.selftest {
            self.reading = Some(Reading { latex: "selftest".into(), kind: "math".into() });
        } else {
            self.run_engine();
        }
    }

    fn overlay(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.overlay_logged && ctx.input(|i| i.viewport().inner_rect.is_some()) {
            self.overlay_logged = true;
            let info = ctx.input(|i| i.viewport().clone());
            let inner = info.inner_rect.unwrap();
            let covers = inner.width() >= self.mon.w as f32 - 1.0 && inner.height() >= self.mon.h as f32 - 1.0;
            eprintln!(
                "[overlay] inner_rect={:?} outer_rect={:?} monitor_size={:?} ppp={} covers_monitor={covers} offset=({},{})",
                inner, info.outer_rect, info.monitor_size, ctx.pixels_per_point(), inner.min.x - self.mon.x as f32, inner.min.y - self.mon.y as f32
            );
            eprintln!("[timing] overlay visible {:.0} ms after process start", self.t0.elapsed().as_secs_f64() * 1000.0);
            self.checks.push(("overlay window covers the monitor".into(), covers));
        }
        if ctx.input(|i| i.key_pressed(Key::Escape) || i.viewport().close_requested()) {
            eprintln!("[input] Esc / close on overlay");
            self.quit = true;
        }
        ctx.set_cursor_icon(CursorIcon::Crosshair);
        let (pressed, down, released, hover) =
            ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.hover_pos()));
        if pressed {
            self.press = hover;
        }
        if down || released {
            self.now = hover.or(self.now);
        }
        let origin = Self::origin(&ctx, &self.mon);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let full = Rect::from_min_size(origin, vec2(self.mon.w as f32, self.mon.h as f32));
            let p = ui.painter();
            p.image(self.tex.id(), full, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            p.rect_filled(ui.max_rect(), 0.0, Color32::from_black_alpha(110));
            if let Some(sel) = self.selection() {
                let uv = Rect::from_min_max(
                    pos2((sel.min.x - origin.x) / full.width(), (sel.min.y - origin.y) / full.height()),
                    pos2((sel.max.x - origin.x) / full.width(), (sel.max.y - origin.y) / full.height()),
                );
                p.image(self.tex.id(), sel, uv, Color32::WHITE);
                p.rect_stroke(sel, 0.0, egui::Stroke::new(1.5, Color32::from_rgb(80, 160, 255)), egui::StrokeKind::Outside);
            }
        });
        if released && let Some(sel) = self.selection() {
            self.finish_selection(&ctx, sel);
        }
        if self.selftest && self.overlay_logged && self.frame > 12 && self.phase == Phase::Select {
            self.finish_selection(&ctx, Rect::from_two_pos(pos2(200.0, 150.0), pos2(720.0, 340.0)));
        }
        ctx.request_repaint();
    }

    /// The whole popup: the options, and the output text. Nothing else.
    fn popup(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.popup_frames += 1;
        if self.popup_frames == 3 {
            let info = ctx.input(|i| i.viewport().clone());
            eprintln!("[popup] inner_rect={:?} outer_rect={:?} (asked for {}x{})", info.inner_rect, info.outer_rect, POPUP_SIZE.0, POPUP_SIZE.1);
            eprintln!("[timing] popup visible {:.0} ms after process start", self.t0.elapsed().as_secs_f64() * 1000.0);
        }
        if ctx.input(|i| i.key_pressed(Key::Escape) || i.viewport().close_requested()) {
            self.quit = true;
        }
        if let Some(rx) = &self.rx
            && let Ok(msg) = rx.try_recv()
        {
            match msg {
                Ok(r) => self.reading = Some(r),
                Err(e) => self.error = Some(e),
            }
            self.rx = None;
        }
        let mut changed_options = false;
        let mut engine_changed = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("wrap");
                for (w, label) in [(Wrap::None, "none"), (Wrap::Inline, "$ $"), (Wrap::Display, "\\[ \\]")] {
                    if ui.selectable_label(self.wrap == w, label).clicked() {
                        self.wrap = w;
                        changed_options = true;
                    }
                }
                ui.separator();
                ui.label("engine");
                for e in ["auto", "small", "vlm"] {
                    if ui.selectable_label(self.engine == e, e).clicked() && self.engine != e {
                        self.engine = e.into();
                        engine_changed = true;
                    }
                }
            });
            let mut text = match (&self.error, self.rx.is_some()) {
                (Some(e), _) => e.clone(),
                (None, true) => "...".to_string(),
                (None, false) => self.display_text(),
            };
            ui.add_sized(ui.available_size(), egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace));
        });
        if engine_changed {
            self.run_engine();
        }
        // the result goes to the clipboard on its own, and again whenever an option changes it
        let shown = self.display_text();
        if !shown.is_empty() && self.rx.is_none() && (shown != self.last_copied || changed_options) {
            ctx.copy_text(shown.clone());
            self.last_copied = shown;
        }
        if self.rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        if self.selftest
            && let Some(since) = self.popup_since
            && since.elapsed() > Duration::from_millis(1500)
        {
            let inner = ctx.input(|i| i.viewport().inner_rect);
            if let Some(r) = inner {
                eprintln!("[popup] after 1.5 s: inner_rect={:?}", r);
                let ok = (r.width() - POPUP_SIZE.0).abs() <= 2.0 && (r.height() - POPUP_SIZE.1).abs() <= 2.0;
                self.checks.push((format!("popup kept the requested size {}x{}", POPUP_SIZE.0, POPUP_SIZE.1), ok));
                let c = r.center();
                let centred = (c.x - (self.mon.x as f32 + self.mon.w as f32 / 2.0)).abs() <= 3.0;
                self.checks.push(("popup is horizontally centred on the monitor".into(), centred));
            }
            self.popup_since = None;
            self.quit = true;
        }
    }
}

/// Test pattern: every pixel has a colour computed from its coordinates.
fn pattern_pixel(x: u32, y: u32) -> image::Rgba<u8> {
    image::Rgba([(x % 251) as u8, (y % 241) as u8, ((x / 7 + y / 5) % 256) as u8, 255])
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array() // the 1x1 root window must be invisible
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        if self.frame == 1 {
            eprintln!("[timing] root first frame {:.0} ms after process start", self.t0.elapsed().as_secs_f64() * 1000.0);
        }
        if self.selftest && self.frame > 400 {
            self.checks.push(("finished in time".into(), false));
            self.quit = true;
        }
        if self.quit {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match self.phase {
            // the root window is the overlay (a first window gets exactly the frame we ask for)
            Phase::Select => self.overlay(ui),
            Phase::Popup => {
                let pos = pos2(
                    self.mon.x as f32 + (self.mon.w as f32 - POPUP_SIZE.0) / 2.0,
                    self.mon.y as f32 + (self.mon.h as f32 - POPUP_SIZE.1) / 2.0,
                );
                let builder = ViewportBuilder::default()
                    .with_title("texcopy")
                    .with_inner_size(vec2(POPUP_SIZE.0, POPUP_SIZE.1))
                    .with_position(pos)
                    .with_resizable(true)
                    // no minimize/zoom buttons: tiling window managers (AeroSpace) treat such a
                    // window as a dialog and leave it where we put it
                    .with_minimize_button(false)
                    .with_maximize_button(false);
                ctx.show_viewport_immediate(ViewportId::from_hash_of("popup"), builder, |ui, _class: ViewportClass| self.popup(ui));
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let t0 = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let selftest = args.iter().any(|a| a == "--selftest");
    let use_engine = !args.iter().any(|a| a == "--no-engine");

    // capture BEFORE any window exists, so the overlay can never appear in its own screenshot
    let mons = xcap::Monitor::all()?;
    let mon = mons.iter().find(|m| m.is_primary().unwrap_or(false)).or(mons.first()).expect("no monitor");
    let (mx, my, mw, mh) = (mon.x()?, mon.y()?, mon.width()?, mon.height()?);
    let (shot, t_capture) = if selftest {
        let scale = mon.scale_factor().unwrap_or(2.0);
        let (pw, ph) = ((mw as f32 * scale).round() as u32, (mh as f32 * scale).round() as u32);
        (RgbaImage::from_fn(pw, ph, pattern_pixel), Duration::ZERO)
    } else {
        let t = Instant::now();
        let img = mon.capture_image()?;
        (img, t.elapsed())
    };
    let scale = shot.width() as f32 / mw as f32;
    eprintln!(
        "[capture] {}x{} px for {}x{} pt monitor at ({mx},{my}), scale {scale:.2}, took {:.0} ms{}",
        shot.width(), shot.height(), mw, mh, t_capture.as_secs_f64() * 1000.0,
        if selftest { " (synthetic pattern)" } else { "" }
    );
    if !selftest {
        let mut seen = std::collections::HashSet::new();
        for y in (0..shot.height()).step_by(97) {
            for x in (0..shot.width()).step_by(97) {
                seen.insert(shot.get_pixel(x, y).0);
            }
        }
        eprintln!("[capture] distinct colours sampled: {} {}", seen.len(), if seen.len() < 4 { "(WARNING: looks blank; check Screen Recording permission)" } else { "(real content)" });
    }

    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("texcopy")
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true)
            .with_position(pos2(mx as f32, my as f32))
            .with_inner_size(vec2(mw as f32, mh as f32)),
        ..Default::default()
    };
    let result = eframe::run_native(
        "texcopy",
        options,
        Box::new(|cc| {
            let img = egui::ColorImage::from_rgba_unmultiplied([shot.width() as usize, shot.height() as usize], shot.as_raw());
            let tex = cc.egui_ctx.load_texture("shot", img, egui::TextureOptions::LINEAR);
            Ok(Box::new(App {
                t0,
                shot,
                mon: Mon { x: mx, y: my, w: mw, h: mh },
                scale,
                tex,
                phase: Phase::Select,
                press: None,
                now: None,
                crop_png: None,
                rx: None,
                reading: None,
                error: None,
                wrap: Wrap::None,
                engine: "auto".into(),
                last_copied: String::new(),
                selftest,
                use_engine,
                frame: 0,
                overlay_logged: false,
                popup_since: None,
                popup_frames: 0,
                checks: Vec::new(),
                quit: false,
                expected: None,
            }))
        }),
    );
    eprintln!("[done] total {:.1} s", t0.elapsed().as_secs_f64());
    result.map_err(|e| anyhow::anyhow!("{e}"))
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(p) = &self.crop_png {
            let _ = std::fs::remove_file(p);
        }
        if self.selftest {
            if let Some((x, y, w, h)) = self.expected {
                let (ew, eh) = ((520.0 * self.scale).round() as i64, (190.0 * self.scale).round() as i64);
                self.checks.push((format!("crop {w}x{h} at ({x},{y}) matches the dragged size (~{ew}x{eh}) and origin ({},{})", (200.0 * self.scale) as u32, (150.0 * self.scale) as u32),
                    (w as i64 - ew).abs() <= 2 && (h as i64 - eh).abs() <= 2 && x == (200.0 * self.scale) as u32 && y == (150.0 * self.scale) as u32));
            }
            let mut ok = !self.checks.is_empty();
            for (name, pass) in &self.checks {
                eprintln!("[selftest] {} {}", if *pass { "PASS" } else { "FAIL" }, name);
                ok &= *pass;
            }
            eprintln!("[selftest] RESULT: {}", if ok { "PASS" } else { "FAIL" });
        }
    }
}
