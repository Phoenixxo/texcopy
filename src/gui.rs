//! the window, with the UI thread, one worker that owns the engines and a thread per capture click.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::Result;
use eframe::egui::{self, Key, ViewportBuilder, ViewportCommand, vec2};
use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::{
    capture, clipboard,
    engine::{self, Engine, Options, Outcome, Warm, Wrap},
    model, prose, tidy,
    ui::{self, Event},
    vlm,
};

const WINDOW_SIZE: (f32, f32) = (520.0, 230.0);
/// time for the window to disappear before the selector starts.
const HIDE_DELAY: Duration = Duration::from_millis(250);
/// after this long idle the helper stops and the models unload.
const IDLE_BEFORE_COOLING: Duration = Duration::from_secs(150);

// settings, remembered between runs.

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    wrap: Wrap,
    engine: Engine,
    /// user said not now to the vision download.
    setup_declined: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wrap: Wrap::None,
            engine: Engine::Auto,
            setup_declined: false,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "texcopy")?;
        Some(dirs.config_dir().join("settings.json"))
    }

    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            std::fs::write(path, text).ok();
        }
    }
}

// messages between threads.

enum Job {
    Read(DynamicImage, Engine),
    /// start the vision engine in the background.
    Warm(Engine),
    Install,
    Quit,
}

/// a finished read.
struct Done {
    latex: String,
    kind: tidy::Kind,
    engine: &'static str,
    /// short things worth a second look.
    notes: Vec<String>,
}

#[derive(Clone)]
struct Progress {
    step: String,
    label: String,
    done: u64,
    total: Option<u64>,
    bytes_per_sec: f64,
}

enum Msg {
    Captured(Result<Option<DynamicImage>, String>),
    Read(Result<Done, String>),
    Progress(Progress),
    Installed(Result<(), String>),
}

// the worker.

fn spawn_worker(
    base: Options,
    ctx: egui::Context,
    tx: Sender<Msg>,
) -> (Sender<Job>, JoinHandle<()>) {
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let handle = std::thread::spawn(move || {
        let mut warm = Warm::default();
        let send = |m: Msg| {
            let _ = tx.send(m);
            ctx.request_repaint();
        };
        loop {
            match job_rx.recv_timeout(IDLE_BEFORE_COOLING) {
                Ok(Job::Read(img, engine)) => {
                    let opts = Options {
                        engine,
                        quiet: true,
                        ..base.clone()
                    };
                    let t = Instant::now();
                    let mut progress = progress_reporter(&send);
                    let result = engine::read(&opts, &img, &mut warm, &mut progress)
                        .map_err(|e| format!("{e:#}"))
                        .and_then(describe);
                    eprintln!("[read] {:.2}s", t.elapsed().as_secs_f32());
                    send(Msg::Read(result));
                }
                Ok(Job::Warm(engine)) => {
                    let opts = Options {
                        engine,
                        quiet: true,
                        ..base.clone()
                    };
                    let t = Instant::now();
                    match warm.prewarm(&opts) {
                        Ok(true) => eprintln!(
                            "[warm] vision engine ready ({:.1}s)",
                            t.elapsed().as_secs_f32()
                        ),
                        Ok(false) => {}
                        Err(e) => eprintln!("[warm] could not start the vision engine: {e:#}"),
                    }
                }
                Ok(Job::Install) => {
                    let mut progress = progress_reporter(&send);
                    let result = vlm::plan(base.llama_server.as_deref(), base.vlm_dir.clone())
                        .and_then(|plan| plan.install(&mut progress))
                        .map(|_| ())
                        .map_err(|e| format!("{e:#}"));
                    send(Msg::Installed(result));
                }
                Ok(Job::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if warm.is_warm() {
                        eprintln!("[idle] cooling down");
                        warm.cool_down();
                    }
                }
            }
        }
        // warm drops here which stops llama-server.
    });
    (job_tx, handle)
}

/// turn setup events into throttled progress messages.
fn progress_reporter<'a>(send: &'a impl Fn(Msg)) -> impl FnMut(Event<'_>) + 'a {
    let mut step = String::new();
    let (mut started, mut base, mut last_sent) = (Instant::now(), 0u64, Instant::now());
    move |e| match e {
        Event::Step { k, of, what } => {
            step = if of > 1 {
                format!("{what} ({k} of {of})")
            } else {
                what.to_string()
            };
            send(Msg::Progress(Progress {
                step: step.clone(),
                label: String::new(),
                done: 0,
                total: None,
                bytes_per_sec: 0.0,
            }));
        }
        Event::Resuming { at, .. } => base = at,
        Event::Bytes { label, done, total } => {
            if done == base || last_sent.elapsed() > Duration::from_millis(120) {
                if done == base {
                    started = Instant::now();
                }
                last_sent = Instant::now();
                let secs = started.elapsed().as_secs_f64();
                let rate = if secs > 0.3 {
                    done.saturating_sub(base) as f64 / secs
                } else {
                    0.0
                };
                send(Msg::Progress(Progress {
                    step: step.clone(),
                    label: label.to_string(),
                    done,
                    total,
                    bytes_per_sec: rate,
                }));
            }
        }
        Event::Verifying { label } => send(Msg::Progress(Progress {
            step: format!("Checking {label}"),
            label: String::new(),
            done: 0,
            total: None,
            bytes_per_sec: 0.0,
        })),
        Event::Unpacking => send(Msg::Progress(Progress {
            step: "Unpacking".into(),
            label: String::new(),
            done: 0,
            total: None,
            bytes_per_sec: 0.0,
        })),
        Event::Finished { .. } => base = 0,
    }
}

/// turn an engine result into what the window shows.
fn describe(o: Outcome) -> Result<Done, String> {
    if o.stop == model::Stop::Looped {
        return Err(
            "Couldn't read this: the model got stuck repeating itself. Try selecting a smaller or cleaner region."
                .into(),
        );
    }
    let mut notes = Vec::new();
    if o.stop == model::Stop::MaxTokens {
        notes.push("The output is probably cut off.".to_string());
    }
    let unsure = o
        .tokens
        .iter()
        .filter(|t| t.prob < model::LOW_CONFIDENCE)
        .count();
    if unsure > 0 {
        notes.push(format!("Unsure about {unsure} symbol(s); worth checking."));
    }
    if o.kind == tidy::Kind::Prose && prose::has_run_together_words(&o.latex) {
        notes.push("Some words look run together. Try one line at a time.".to_string());
    }
    if o.engine == "small" && o.latex.contains("\\nu") {
        notes.push("Read a \\nu. In italics, v and \u{3bd} look alike; check which it is.".into());
    }
    Ok(Done {
        latex: o.latex,
        kind: o.kind,
        engine: o.engine,
        notes,
    })
}

// the window.

#[derive(Clone)]
struct SetupItem {
    what: String,
    size: u64,
}

struct App {
    settings: Settings,
    msg_rx: Receiver<Msg>,
    msg_tx: Sender<Msg>,
    job_tx: Sender<Job>,
    worker: Option<JoinHandle<()>>,
    base: Options,
    /// the last picture captured, so changing the engine can read it again.
    shot: Option<DynamicImage>,
    capturing: bool,
    reading: bool,
    installing: Option<Progress>,
    done: Option<Done>,
    error: Option<String>,
    /// what the vision engine would still download, None when it's ready.
    setup: Option<Vec<SetupItem>>,
    last_copied: String,
    quit: bool,
    test: Option<SelfTest>,
}

/// --gui-selftest <png> pretends the user captured this picture and prints PASS or FAIL.
struct SelfTest {
    picture: PathBuf,
    frame: u32,
    frames_while_hidden: u32,
    read_started: Option<Instant>,
    /// how long each read took in seconds.
    read_secs: Vec<f32>,
    checks: Vec<(String, bool)>,
    /// TEXCOPY_GUI_SCREENSHOT=<png> saves a picture of the window, never the desktop.
    screenshot: Option<PathBuf>,
    settled_frames: u32,
    screenshot_requested: bool,
    screenshot_saved: bool,
}

impl App {
    fn refresh_setup(&mut self) {
        self.setup = vlm::plan(self.base.llama_server.as_deref(), self.base.vlm_dir.clone())
            .ok()
            .filter(|p| !p.is_ready())
            .map(|p| {
                p.items()
                    .into_iter()
                    .map(|i| SetupItem {
                        what: i.what,
                        size: i.size,
                    })
                    .collect()
            });
    }

    fn shown_text(&self) -> String {
        self.done.as_ref().map_or(String::new(), |d| {
            engine::wrapped(self.settings.wrap, d.kind, &d.latex)
        })
    }

    /// ask the worker to load the vision engine now.
    fn warm_up(&self) {
        if std::env::var_os("TEXCOPY_GUI_NO_PREWARM").is_none() {
            let _ = self.job_tx.send(Job::Warm(self.settings.engine));
        }
    }

    fn busy(&self) -> bool {
        self.capturing || self.reading || self.installing.is_some()
    }

    /// hide the window, let the user drag, bring it back.
    fn start_capture(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        self.capturing = true;
        self.done = None;
        self.error = None;
        // the engine loads while the user is still dragging.
        self.warm_up();
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        let (tx, ctx) = (self.msg_tx.clone(), ctx.clone());
        let fake = self.test.as_ref().map(|t| t.picture.clone());
        std::thread::spawn(move || {
            std::thread::sleep(HIDE_DELAY);
            let result = match fake {
                Some(p) => {
                    // self test only, stands in for the time spent dragging.
                    let ms = std::env::var("TEXCOPY_SELFTEST_SELECT_MS")
                        .ok()
                        .and_then(|v| v.parse().ok());
                    std::thread::sleep(Duration::from_millis(ms.unwrap_or(0)));
                    image::open(p).map(Some).map_err(|e| e.to_string())
                }
                None => capture::interactive().map_err(|e| format!("{e:#}")),
            };
            let _ = tx.send(Msg::Captured(result));
            ctx.request_repaint();
        });
    }

    fn start_read(&mut self) {
        let Some(img) = self.shot.clone() else { return };
        // the vision engine can only be forced once it's set up.
        if self.settings.engine == Engine::Vlm && self.setup.is_some() {
            return;
        }
        self.reading = true;
        self.done = None;
        self.error = None;
        if let Some(t) = self.test.as_mut() {
            t.read_started = Some(Instant::now());
        }
        let _ = self.job_tx.send(Job::Read(img, self.settings.engine));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                Msg::Captured(result) => {
                    self.capturing = false;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                    match result {
                        Ok(Some(img)) => {
                            self.shot = Some(img);
                            self.start_read();
                        }
                        Ok(None) => {} // cancelled with esc.
                        Err(e) => self.error = Some(e),
                    }
                }
                Msg::Read(result) => {
                    self.reading = false;
                    self.installing = None; // a first run small model download shows progress too.
                    if let Some(t) = self.test.as_mut()
                        && let Some(start) = t.read_started.take()
                    {
                        t.read_secs.push(start.elapsed().as_secs_f32());
                    }
                    match result {
                        Ok(d) => {
                            eprintln!("[engine] {}", d.engine);
                            self.done = Some(d);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Msg::Progress(p) => self.installing = Some(p),
                Msg::Installed(result) => {
                    self.installing = None;
                    if let Err(e) = result {
                        self.error = Some(e);
                    }
                    self.refresh_setup();
                    self.start_read(); // a capture waiting on the engine can go ahead.
                }
            }
        }
    }

    /// write the picture the self test asked for.
    fn save_screenshot(&mut self, ctx: &egui::Context) {
        let Some(t) = self.test.as_mut() else { return };
        let Some(path) = t.screenshot.clone() else {
            return;
        };
        let picture = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = picture {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            match image::RgbaImage::from_raw(img.width() as u32, img.height() as u32, rgba) {
                Some(out) if out.save(&path).is_ok() => {
                    eprintln!("[screenshot] saved {}", path.display())
                }
                _ => eprintln!("[screenshot] could not save {}", path.display()),
            }
            t.screenshot_saved = true;
        }
    }

    fn step_selftest(&mut self, ctx: &egui::Context) {
        if self.quit {
            return;
        }
        let Some(t) = self.test.as_mut() else { return };
        t.frame += 1;
        if self.capturing {
            t.frames_while_hidden += 1;
        }
        let (frame, hidden, reads) = (t.frame, t.frames_while_hidden, t.read_secs.len());
        if t.screenshot.is_some() && reads == 1 && !t.screenshot_saved {
            // draw the result for a few frames, then ask for a picture.
            t.settled_frames += 1;
            if t.settled_frames == 8 && !t.screenshot_requested {
                t.screenshot_requested = true;
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            }
            return;
        }
        // two captures in a row, the second reuses the helper.
        if (frame == 10 || (reads == 1 && frame > 10)) && !self.busy() {
            self.start_capture(ctx);
        }
        if (reads >= 2 && !self.busy()) || frame > 1500 {
            let text = self.shown_text();
            let t = self.test.as_mut().expect("checked above");
            let secs = t.read_secs.clone();
            eprintln!("[selftest] read times: {secs:?}");
            t.checks
                .push(("the window kept running while hidden".into(), hidden >= 2));
            t.checks.push((
                "the engine returned the sentence".into(),
                text.contains("cut-vertex"),
            ));
            t.checks.push((
                "the second read reused the warm engine (under 4 s)".into(),
                secs.get(1).is_some_and(|&s| s < 4.0),
            ));
            eprintln!("[selftest] output: {text}");
            self.quit = true;
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if self.test.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30)); // the self test counts frames.
        }
        self.save_screenshot(ctx);
        self.step_selftest(ctx);
        if self.quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::Escape) || i.viewport().close_requested()) {
            self.quit = true;
        }
        let mut capture = ctx.input(|i| i.key_pressed(Key::C) && !i.modifiers.any())
            && !ctx.egui_wants_keyboard_input();
        let (mut options_changed, mut engine_changed) = (false, false);
        let (mut install, mut decline) = (false, false);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                capture |= ui.button("Capture").clicked();
                ui.separator();
                ui.label("wrap");
                for (w, label) in [
                    (Wrap::None, "none"),
                    (Wrap::Inline, "$ $"),
                    (Wrap::Display, "\\[ \\]"),
                ] {
                    if ui
                        .selectable_label(self.settings.wrap == w, label)
                        .clicked()
                    {
                        self.settings.wrap = w;
                        options_changed = true;
                    }
                }
                ui.separator();
                ui.label("engine");
                for (e, label) in [
                    (Engine::Auto, "auto"),
                    (Engine::Small, "small"),
                    (Engine::Vlm, "vlm"),
                ] {
                    if ui
                        .selectable_label(self.settings.engine == e, label)
                        .clicked()
                        && self.settings.engine != e
                    {
                        self.settings.engine = e;
                        options_changed = true;
                        engine_changed = true;
                    }
                }
            });

            let show_card = self.setup.is_some()
                && self.done.is_none()
                && self.error.is_none()
                && !self.reading
                && !self.capturing
                && (!self.settings.setup_declined || self.settings.engine == Engine::Vlm);
            if let Some(p) = self.installing.clone() {
                install_progress(ui, &p);
            } else if show_card {
                let items = self.setup.clone().unwrap_or_default();
                (install, decline) = setup_card(ui, &items);
            } else {
                let mut text = if let Some(e) = &self.error {
                    e.clone()
                } else if self.reading {
                    "...".to_string()
                } else {
                    self.shown_text()
                };
                let notes = self
                    .done
                    .as_ref()
                    .map(|d| d.notes.clone())
                    .unwrap_or_default();
                let note_height = if notes.is_empty() { 0.0 } else { 20.0 };
                let size = vec2(
                    ui.available_width(),
                    (ui.available_height() - note_height).max(40.0),
                );
                ui.add_sized(
                    size,
                    egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace),
                );
                if !notes.is_empty() {
                    ui.label(egui::RichText::new(notes.join("  ")).small().weak());
                }
            }
        });

        if options_changed {
            self.settings.save();
        }
        if install {
            self.installing = Some(Progress {
                step: "Starting".into(),
                label: String::new(),
                done: 0,
                total: None,
                bytes_per_sec: 0.0,
            });
            self.error = None;
            let _ = self.job_tx.send(Job::Install);
        }
        if decline {
            self.settings.setup_declined = true;
            self.settings.save();
        }
        if capture {
            self.start_capture(&ctx);
        }
        if engine_changed && !self.busy() {
            self.start_read();
        }
        // the result goes to the clipboard on its own and again when an option changes it.
        let shown = self.shown_text();
        if !shown.is_empty() && !self.reading && shown != self.last_copied && self.test.is_none() {
            if let Err(e) = clipboard::copy_text(&shown) {
                self.error = Some(format!("Couldn't copy to the clipboard: {e:#}"));
            }
            self.last_copied = shown;
        }
    }
}

/// the first run offer, returns (install clicked, not now clicked).
fn setup_card(ui: &mut egui::Ui, items: &[SetupItem]) -> (bool, bool) {
    let total: u64 = items.iter().map(|i| i.size).sum();
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Vision engine (optional, much better for sentences)").strong());
    ui.add_space(4.0);
    for i in items {
        ui.label(format!("• {}  ({})", i.what, ui::human_bytes(i.size)));
    }
    ui.add_space(8.0);
    let (mut install, mut decline) = (false, false);
    ui.horizontal(|ui| {
        install = ui
            .button(format!("Download {}", ui::human_bytes(total)))
            .clicked();
        decline = ui.button("Not now").clicked();
    });
    (install, decline)
}

/// the download in progress.
fn install_progress(ui: &mut egui::Ui, p: &Progress) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(&p.step).strong());
    if let Some(total) = p.total.filter(|&t| t > 0) {
        let frac = (p.done as f32 / total as f32).clamp(0.0, 1.0);
        let eta = if p.bytes_per_sec > 0.0 {
            format!(
                "  ETA {}",
                ui::human_duration(total.saturating_sub(p.done) as f64 / p.bytes_per_sec)
            )
        } else {
            String::new()
        };
        ui.add(egui::ProgressBar::new(frac).text(format!(
            "{} / {}  {}/s{eta}",
            ui::human_bytes(p.done),
            ui::human_bytes(total),
            ui::human_bytes(p.bytes_per_sec as u64)
        )));
        ui.label(egui::RichText::new(&p.label).small().weak());
    } else {
        ui.add(egui::ProgressBar::new(0.0).animate(true));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // stop the worker and wait, it owns llama-server.
        let _ = self.job_tx.send(Job::Quit);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        if let Some(t) = &self.test {
            let mut ok = !t.checks.is_empty();
            for (name, pass) in &t.checks {
                eprintln!("[selftest] {} {name}", if *pass { "PASS" } else { "FAIL" });
                ok &= *pass;
            }
            eprintln!("[selftest] RESULT: {}", if ok { "PASS" } else { "FAIL" });
        }
    }
}

/// icon for the window, taskbar and dock.
fn window_icon() -> egui::IconData {
    let rgba = image::load_from_memory(include_bytes!("../packaging/icon/icon-256.png"))
        .expect("the embedded icon is a valid PNG")
        .to_rgba8();
    egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

/// open the window, selftest is a picture to use instead of a real selection.
pub fn run(base: Options, selftest: Option<PathBuf>) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("texcopy")
            .with_app_id("texcopy") // matches StartupWMClass in the linux menu entry.
            .with_icon(std::sync::Arc::new(window_icon()))
            .with_inner_size(vec2(WINDOW_SIZE.0, WINDOW_SIZE.1))
            .with_resizable(true)
            // no minimize or zoom buttons, tiling window managers treat those as dialogs and leave them floating.
            .with_minimize_button(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    eframe::run_native(
        "texcopy",
        options,
        Box::new(move |cc| {
            let (msg_tx, msg_rx) = mpsc::channel();
            let (job_tx, worker) = spawn_worker(base.clone(), cc.egui_ctx.clone(), msg_tx.clone());
            let mut app = App {
                settings: Settings::load(),
                msg_rx,
                msg_tx,
                job_tx,
                worker: Some(worker),
                base,
                shot: None,
                capturing: false,
                reading: false,
                installing: None,
                done: None,
                error: None,
                setup: None,
                last_copied: String::new(),
                quit: false,
                test: selftest.map(|picture| SelfTest {
                    picture,
                    frame: 0,
                    frames_while_hidden: 0,
                    read_started: None,
                    read_secs: Vec::new(),
                    checks: Vec::new(),
                    screenshot: std::env::var_os("TEXCOPY_GUI_SCREENSHOT").map(PathBuf::from),
                    settled_frames: 0,
                    screenshot_requested: false,
                    screenshot_saved: false,
                }),
            };
            app.refresh_setup();
            app.warm_up();
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn progress_of(msgs: &[Msg]) -> Vec<Progress> {
        msgs.iter()
            .filter_map(|m| match m {
                Msg::Progress(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn download_events_become_throttled_progress_with_a_speed() {
        let sent = RefCell::new(Vec::new());
        let send = |m: Msg| sent.borrow_mut().push(m);
        let mut report = progress_reporter(&send);
        report(Event::Step {
            k: 2,
            of: 2,
            what: "Downloading the model",
        });
        report(Event::Bytes {
            label: "model.gguf",
            done: 0,
            total: Some(1000),
        });
        // a burst of chunks inside the throttle isn't forwarded one by one.
        for done in [10, 20, 30, 40] {
            report(Event::Bytes {
                label: "model.gguf",
                done,
                total: Some(1000),
            });
        }
        std::thread::sleep(Duration::from_millis(400));
        report(Event::Bytes {
            label: "model.gguf",
            done: 500,
            total: Some(1000),
        });
        report(Event::Verifying {
            label: "model.gguf",
        });
        drop(report);

        let p = progress_of(&sent.borrow());
        assert_eq!(
            p.len(),
            4,
            "step, first chunk, one throttled update, verify"
        );
        assert_eq!(p[0].step, "Downloading the model (2 of 2)");
        assert_eq!(p[1].done, 0);
        assert_eq!((p[2].done, p[2].total), (500, Some(1000)));
        assert!(p[2].bytes_per_sec > 0.0, "a speed once enough time passed");
        assert_eq!(p[2].label, "model.gguf");
        assert_eq!(p[3].step, "Checking model.gguf");
    }

    #[test]
    fn a_stuck_read_becomes_an_error_and_a_clean_one_shows_no_notes() {
        let outcome = |stop, latex: &str| Outcome {
            latex: latex.into(),
            kind: tidy::Kind::Math,
            raw: String::new(),
            engine: "small",
            stop,
            min_prob: None,
            mean_logprob: None,
            tokens: Vec::new(),
        };
        assert!(describe(outcome(model::Stop::Looped, "x")).is_err());
        let ok = describe(outcome(model::Stop::Eos, "x^2")).unwrap();
        assert!(ok.notes.is_empty());
        let cut = describe(outcome(model::Stop::MaxTokens, "x^2")).unwrap();
        assert!(cut.notes[0].contains("cut off"));
        let nu = describe(outcome(model::Stop::Eos, "\\nu")).unwrap();
        assert!(nu.notes[0].contains("\\nu"));
    }
}
