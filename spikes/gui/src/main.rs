//! Phase 2 spike, round 2: a small window first; click Capture, drag over something, get the LaTeX.
//! See FINDINGS.md next to this file for what the first round (a custom full-screen overlay) taught us.
//!
//! Run from this folder:  cargo run --release     (needs ../../target/release/texcopy built)
//!
//!   gui-spike              open the window; click Capture (or press C) and drag over some maths
//!   gui-spike --selftest   no mouse needed: captures a fixture image instead, prints PASS/FAIL
//!
//! The window is the whole interface: the capture button, the options, and the output text.
//! On macOS the selection itself is done by the system's own `screencapture -i` (the crosshair you
//! know from Cmd-Shift-4). That keeps window managers, the menu bar and multiple monitors out of
//! our hands. Windows and Linux will need their own selector; see FINDINGS.md.

use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

use eframe::egui::{self, Key, ViewportBuilder, ViewportCommand, vec2};

const WINDOW_SIZE: (f32, f32) = (520.0, 230.0);
/// Time for the window to disappear before the selector starts, so it is not in the picture.
const HIDE_DELAY: Duration = Duration::from_millis(250);

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

/// What the capture thread reports back.
enum Captured {
    Image,
    Cancelled,
    Failed(String),
}

struct App {
    t0: Instant,
    capture_rx: Option<mpsc::Receiver<Captured>>,
    engine_rx: Option<mpsc::Receiver<Result<Reading, String>>>,
    shot_png: PathBuf,
    have_shot: bool,
    reading: Option<Reading>,
    error: Option<String>,
    wrap: Wrap,
    engine: String,
    last_copied: String,
    selftest: bool,
    frame: u32,
    checks: Vec<(String, bool)>,
    frames_while_capturing: u32,
    quit: bool,
}

impl App {
    fn display_text(&self) -> String {
        let Some(r) = &self.reading else { return String::new() };
        // a sentence carries its own $...$ marks; wrapping only applies to a formula
        match (self.wrap, r.kind.as_str()) {
            (Wrap::None, _) | (_, "prose") => r.latex.clone(),
            (Wrap::Inline, _) => format!("${}$", r.latex),
            (Wrap::Display, _) => format!("\\[ {} \\]", r.latex),
        }
    }

    fn capturing(&self) -> bool {
        self.capture_rx.is_some()
    }

    /// Hide the window, let the user drag a region, then bring the window back with the result.
    fn start_capture(&mut self, ctx: &egui::Context) {
        if self.capturing() {
            return;
        }
        self.reading = None;
        self.error = None;
        std::fs::remove_file(&self.shot_png).ok(); // so "no file afterwards" means "cancelled"
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        let (tx, rx) = mpsc::channel();
        self.capture_rx = Some(rx);
        let (ctx, path) = (ctx.clone(), self.shot_png.clone());
        let fake = std::env::var("TEXCOPY_FAKE_CAPTURE").ok().filter(|_| self.selftest);
        std::thread::spawn(move || {
            std::thread::sleep(HIDE_DELAY);
            let msg = match fake {
                // self-test: pretend the user dragged over this picture
                Some(src) => std::fs::copy(src, &path).map(|_| Captured::Image).unwrap_or_else(|e| Captured::Failed(e.to_string())),
                None => system_selector(&path),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn run_engine(&mut self) {
        if !self.have_shot {
            return;
        }
        self.reading = None;
        self.error = None;
        let (tx, rx) = mpsc::channel();
        self.engine_rx = Some(rx);
        let (engine, path) = (self.engine.clone(), self.shot_png.clone());
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

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.capture_rx {
            self.frames_while_capturing += 1;
            if let Ok(msg) = rx.try_recv() {
                self.capture_rx = None;
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                eprintln!("[timing] window back {:.0} ms after process start", self.t0.elapsed().as_secs_f64() * 1000.0);
                match msg {
                    Captured::Image => {
                        self.have_shot = true;
                        self.run_engine();
                    }
                    Captured::Cancelled => {}
                    Captured::Failed(e) => self.error = Some(e),
                }
            }
        }
        if let Some(rx) = &self.engine_rx
            && let Ok(msg) = rx.try_recv()
        {
            match msg {
                Ok(r) => self.reading = Some(r),
                Err(e) => self.error = Some(e),
            }
            self.engine_rx = None;
        }
    }

    fn selftest_step(&mut self, ctx: &egui::Context) {
        if self.frame == 10 && !self.capturing() {
            self.start_capture(ctx);
        }
        if self.frame > 600 {
            self.checks.push(("finished in time".into(), false));
            self.quit = true;
        }
        if self.have_shot && self.engine_rx.is_none() && !self.capturing() && !self.quit {
            let text = self.display_text();
            self.checks.push(("the update loop kept running while the window was hidden".into(), self.frames_while_capturing >= 2));
            self.checks.push(("the engine produced text after the capture".into(), text.contains("cut") || text.contains("vertex")));
            eprintln!("[selftest] output: {text}");
            self.quit = true;
        }
    }
}

/// The system's own region selector, writing a PNG. Esc leaves no file, which means "cancelled".
#[cfg(target_os = "macos")]
fn system_selector(path: &std::path::Path) -> Captured {
    // -i interactive (drag a region, Esc cancels), -x no camera sound
    match Command::new("screencapture").args(["-i", "-x"]).arg(path).status() {
        Ok(s) if s.success() && path.exists() => Captured::Image,
        Ok(_) => Captured::Cancelled,
        Err(e) => Captured::Failed(format!("could not run screencapture: {e}")),
    }
}

#[cfg(not(target_os = "macos"))]
fn system_selector(_path: &std::path::Path) -> Captured {
    Captured::Failed("this spike only has a selector for macOS so far".into())
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        if self.frame == 1 {
            eprintln!("[timing] first frame {:.0} ms after process start", self.t0.elapsed().as_secs_f64() * 1000.0);
        }
        self.poll(ctx);
        if self.selftest {
            self.selftest_step(ctx);
        }
        if self.quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if self.capturing() || self.engine_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    /// The whole interface: capture button, the options, and the output text. Nothing else.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::Escape) || i.viewport().close_requested()) && !self.capturing() {
            self.quit = true;
        }
        let mut changed_options = false;
        let mut engine_changed = false;
        let mut capture = ctx.input(|i| i.key_pressed(Key::C) && !i.modifiers.any()) && !ctx.egui_wants_keyboard_input();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                capture |= ui.button("Capture").clicked();
                ui.separator();
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
            let mut text = match (&self.error, self.engine_rx.is_some()) {
                (Some(e), _) => e.clone(),
                (None, true) => "...".to_string(),
                (None, false) => self.display_text(),
            };
            ui.add_sized(ui.available_size(), egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace));
        });
        if capture {
            self.start_capture(&ctx);
        }
        if engine_changed {
            self.run_engine();
        }
        // the result goes to the clipboard on its own, and again whenever an option changes it
        let shown = self.display_text();
        if !shown.is_empty() && self.engine_rx.is_none() && (shown != self.last_copied || changed_options) {
            ctx.copy_text(shown.clone());
            self.last_copied = shown;
        }
    }
}

fn main() -> anyhow::Result<()> {
    let t0 = Instant::now();
    let selftest = std::env::args().any(|a| a == "--selftest");
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("texcopy")
            .with_inner_size(vec2(WINDOW_SIZE.0, WINDOW_SIZE.1))
            .with_resizable(true)
            // no minimize/zoom buttons: tiling window managers (AeroSpace) treat such a window as
            // a dialog and leave it where we put it, instead of tiling it
            .with_minimize_button(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    let result = eframe::run_native(
        "texcopy",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(App {
                t0,
                capture_rx: None,
                engine_rx: None,
                shot_png: std::env::temp_dir().join(format!("texcopy-gui-{}.png", std::process::id())),
                have_shot: false,
                reading: None,
                error: None,
                wrap: Wrap::None,
                engine: "auto".into(),
                last_copied: String::new(),
                selftest,
                frame: 0,
                checks: Vec::new(),
                frames_while_capturing: 0,
                quit: false,
            }))
        }),
    );
    eprintln!("[done] total {:.1} s", t0.elapsed().as_secs_f64());
    result.map_err(|e| anyhow::anyhow!("{e}"))
}

impl Drop for App {
    fn drop(&mut self) {
        std::fs::remove_file(&self.shot_png).ok();
        if self.selftest {
            let mut ok = !self.checks.is_empty();
            for (name, pass) in &self.checks {
                eprintln!("[selftest] {} {}", if *pass { "PASS" } else { "FAIL" }, name);
                ok &= *pass;
            }
            eprintln!("[selftest] RESULT: {}", if ok { "PASS" } else { "FAIL" });
        }
    }
}
