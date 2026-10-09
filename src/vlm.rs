//! The vision-language engine: OvisOCR2 (0.8B, Apache-2.0) served by a short-lived `llama-server`.
//!
//! texcopy starts `llama-server` on a free local port, sends the image, reads the Markdown the model
//! writes, and stops the server again. Nothing stays in memory between runs. The model reads whole
//! paragraphs and very long equations, which the small formula model cannot.
//!
//! Needs two things: the `llama-server` program from llama.cpp (found on PATH, or pass
//! `--llama-server`), and the model files (1.0 GB, downloaded by `--engine vlm`).

use std::{
    fs::{self, File},
    io::Cursor,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use image::{DynamicImage, ImageFormat, imageops::FilterType};

use crate::{
    fetch::{self, Download},
    helper, prose,
    ui::{self, Event, Reporter},
};

const BASE_URL: &str = "https://huggingface.co/bartowski/ATH-MaaS_OvisOCR2-GGUF/resolve/main";

struct RemoteFile {
    /// name on Hugging Face
    remote: &'static str,
    /// name we store it under
    local: &'static str,
    label: &'static str,
    size: u64,
    sha256: &'static str,
}

const MODEL_FILES: [RemoteFile; 2] = [
    RemoteFile {
        remote: "ATH-MaaS_OvisOCR2-Q8_0.gguf",
        local: "model.gguf",
        label: "OvisOCR2 model",
        size: 813_576_320,
        sha256: "8ca48f886bcd5f33636be25c935f07e0d2a83e839d9bce29c7fd3e4bfb344d33",
    },
    RemoteFile {
        remote: "mmproj-ATH-MaaS_OvisOCR2-f16.gguf",
        local: "mmproj.gguf",
        label: "OvisOCR2 vision part",
        size: 204_987_040,
        sha256: "4e0e9cb9d79dd0f423ba152a51816aa82a1f1a9d1a0190b6f67b2cd4cc5dd681",
    },
];
const PROMPT: &str = "Extract all readable content from the image in natural human reading order and output the result as a single Markdown document.";
/// Larger screenshots are shrunk to this: the model gains nothing from more pixels and gets slower.
const MAX_SIDE: u32 = 2000;
const MAX_TOKENS: u32 = 2048;

fn model_download_bytes() -> u64 {
    MODEL_FILES.iter().map(|f| f.size).sum()
}

pub struct Files {
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

/// Where the model lives: an explicit directory, else `<cache>/ovisocr2`.
pub fn model_dir(over: Option<PathBuf>) -> Option<PathBuf> {
    over.or_else(|| fetch::cache_dir().map(|d| d.join("ovisocr2")))
}

pub fn files_in(dir: &Path) -> Option<Files> {
    let files = Files {
        model: dir.join(MODEL_FILES[0].local),
        mmproj: dir.join(MODEL_FILES[1].local),
    };
    (files.model.is_file() && files.mmproj.is_file()).then_some(files)
}

/// Download whatever is missing from `dir`, checksum-verified, with progress.
fn download_model(dir: &Path, report: Reporter) -> Result<Files> {
    fs::create_dir_all(dir)?;
    for f in &MODEL_FILES {
        fetch::download_verified(
            &Download {
                url: &format!("{BASE_URL}/{}", f.remote),
                dest: &dir.join(f.local),
                label: f.label,
                size: Some(f.size),
                sha256: Some(f.sha256),
            },
            report,
        )?;
    }
    files_in(dir).context("download finished but the files are missing")
}

/// `llama-server`: the one texcopy installed (the version it was tested with), else the first on
/// PATH. An explicit `--llama-server` is used as given.
pub fn find_server(over: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = over {
        return p.is_file().then(|| p.to_path_buf());
    }
    helper::installed().or_else(|| {
        let name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        let paths = std::env::var_os("PATH")?;
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

pub const INSTALL_HINT: &str = "texcopy's vision engine needs `llama-server` from llama.cpp:\n  macOS:   brew install llama.cpp\n  Windows: winget install llama.cpp   (or download a release from https://github.com/ggml-org/llama.cpp/releases)\n  Linux:   a release build from the page above, or your package manager\nThen put it on your PATH, or pass --llama-server /path/to/llama-server.";

pub struct Setup {
    pub server: PathBuf,
    pub files: Files,
}

/// The vision engine if it is already set up. Never asks and never downloads.
pub fn ready(server_over: Option<&Path>, dir_over: Option<PathBuf>) -> Option<Setup> {
    let server = find_server(server_over)?;
    let files = files_in(&model_dir(dir_over)?)?;
    Some(Setup { server, files })
}

/// What setting the vision engine up would involve, worked out without touching the network.
/// The terminal prints it and asks `[y/N]`; the window shows it with an Install button.
pub struct Plan {
    server: Option<PathBuf>,
    files: Option<Files>,
    dir: PathBuf,
    release: Option<&'static helper::Release>,
}

/// One thing to download.
pub struct PlanItem {
    pub what: String,
    pub size: u64,
    pub from: &'static str,
    pub to: String,
}

impl Plan {
    /// Nothing left to download: the engine can start.
    pub fn is_ready(&self) -> bool {
        self.release.is_none() && self.files.is_some()
    }

    pub fn total_bytes(&self) -> u64 {
        self.items().iter().map(|i| i.size).sum()
    }

    pub fn items(&self) -> Vec<PlanItem> {
        let mut items = Vec::new();
        if let Some(r) = self.release {
            items.push(PlanItem {
                what: format!("llama.cpp {} helper: {}", helper::TAG, r.describe),
                size: r.size,
                from: "github.com/ggml-org/llama.cpp (MIT licence), checksum-verified",
                to: helper::install_dir().map_or(String::new(), |d| d.display().to_string()),
            });
        }
        if self.files.is_none() {
            items.push(PlanItem {
                what: "OvisOCR2 model (reads text and long equations)".into(),
                size: model_download_bytes(),
                from: "huggingface.co/bartowski/ATH-MaaS_OvisOCR2-GGUF (Apache-2.0), checksum-verified",
                to: self.dir.display().to_string(),
            });
        }
        items
    }

    /// Download and install whatever is missing, reporting progress. Returns the ready engine.
    pub fn install(self, report: Reporter) -> Result<Setup> {
        let n_steps = usize::from(self.release.is_some()) + usize::from(self.files.is_none());
        let mut k = 0;
        let server = match (self.server, self.release) {
            (Some(s), _) => s,
            (None, Some(r)) => {
                k += 1;
                report(Event::Step {
                    k,
                    of: n_steps,
                    what: "Installing the llama.cpp helper",
                });
                helper::install(r, report)?
            }
            (None, None) => unreachable!("a missing server always has a release to install"),
        };
        let files = match self.files {
            Some(f) => f,
            None => {
                k += 1;
                report(Event::Step {
                    k,
                    of: n_steps,
                    what: "Downloading the OvisOCR2 model",
                });
                download_model(&self.dir, report)?
            }
        };
        Ok(Setup { server, files })
    }
}

/// Work out what the vision engine still needs. Errors when there is nowhere to put it, or no
/// helper build exists for this machine.
pub fn plan(server_over: Option<&Path>, dir_over: Option<PathBuf>) -> Result<Plan> {
    if let Some(p) = server_over
        && !p.is_file()
    {
        bail!("--llama-server {} does not exist", p.display());
    }
    let dir = model_dir(dir_over).context("cannot determine a cache directory; pass --vlm-dir")?;
    let server = find_server(server_over);
    let release = if server.is_none() {
        Some(helper::require_release()?)
    } else {
        None
    };
    Ok(Plan {
        server,
        files: files_in(&dir),
        dir,
        release,
    })
}

/// Set the vision engine up from the terminal: explain exactly what will be downloaded, ask once,
/// then do it with progress. With `assume_yes` (--yes) it explains and proceeds without asking.
pub fn ensure(
    server_over: Option<&Path>,
    dir_over: Option<PathBuf>,
    assume_yes: bool,
) -> Result<Setup> {
    let plan = plan(server_over, dir_over)?;
    if plan.is_ready() {
        return plan.install(&mut ui::silent);
    }
    eprintln!("The vision engine (OvisOCR2) needs a one-time setup.");
    eprintln!("Everything runs on your computer; nothing is uploaded.\n");
    for (i, item) in plan.items().iter().enumerate() {
        eprintln!("  {}. {}", i + 1, item.what);
        eprintln!("     {:>9}  from {}", ui::human_bytes(item.size), item.from);
        eprintln!("     saves to {}", item.to);
    }
    eprintln!(
        "\n  Total download: {}\n",
        ui::human_bytes(plan.total_bytes())
    );
    if assume_yes {
        eprintln!("(--yes given: continuing without asking)\n");
    } else if !ui::confirm("Download and install now?", false)? {
        bail!(
            "setup cancelled. Nothing was downloaded. (`texcopy --engine small` skips the vision engine.)"
        );
    }
    let installed_helper = plan.release.is_some();
    let mut term = ui::Terminal::default();
    let setup = plan.install(&mut |e| term.event(e))?;
    eprintln!("\nSetup complete.");
    if cfg!(target_os = "macos") && installed_helper {
        eprintln!("(The first run can take a little longer while macOS checks the new helper.)");
    }
    eprintln!();
    Ok(setup)
}

/// A running `llama-server`. Stopped when dropped.
pub struct Server {
    child: Child,
    port: u16,
    log: PathBuf,
    agent: ureq::Agent,
}

impl Server {
    pub fn start(bin: &Path, files: &Files) -> Result<Self> {
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let log = std::env::temp_dir().join(format!("texcopy-llama-{}.log", std::process::id()));
        let out = File::create(&log)?;
        let child = Command::new(bin)
            .arg("-m")
            .arg(&files.model)
            .arg("--mmproj")
            .arg(&files.mmproj)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // all layers on the GPU if there is one; one slot; and no prompt cache, which otherwise
            // grows the process from 1.4 GB to over 3 GB after a few dozen images
            .args(["-ngl", "99", "-c", "8192", "-np", "1", "--cache-ram", "0"])
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .spawn()
            .with_context(|| format!("starting {}", bin.display()))?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(180)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut server = Server {
            child,
            port,
            log,
            agent,
        };
        server.wait_ready()?;
        Ok(server)
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn log_tail(&self) -> String {
        let text = fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().rev().take(6).collect();
        lines.into_iter().rev().collect::<Vec<_>>().join("\n")
    }

    fn wait_ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                bail!("llama-server exited ({status}):\n{}", self.log_tail());
            }
            // 503 while the model loads, 200 when it is ready
            if self
                .agent
                .get(&self.url("/health"))
                .call()
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        bail!(
            "llama-server was not ready after 120 s:\n{}",
            self.log_tail()
        )
    }

    /// Read the image. Returns the model's Markdown and whether it was cut off at the token limit.
    pub fn read(&self, img: &DynamicImage) -> Result<(String, bool)> {
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", png_base64(img)?)}},
                {"type": "text", "text": PROMPT},
            ]}],
            "temperature": 0.0,
            "max_tokens": MAX_TOKENS,
        });
        let resp = self
            .agent
            .post(&self.url("/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .context("sending the image to llama-server")?;
        let status = resp.status();
        let text = resp.into_body().read_to_string()?;
        if !status.is_success() {
            bail!(
                "llama-server answered {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        let v: serde_json::Value = serde_json::from_str(&text).context("reading the reply")?;
        let choice = &v["choices"][0];
        let content = choice["message"]["content"]
            .as_str()
            .context("the reply had no text")?;
        Ok((content.to_string(), choice["finish_reason"] == "length"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.log);
    }
}

/// PNG, shrunk if huge, flattened onto white (the model expects dark ink on a light page).
fn png_base64(img: &DynamicImage) -> Result<String> {
    let img = if img.width().max(img.height()) > MAX_SIDE {
        img.resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle)
    } else {
        img.clone()
    };
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = f32::from(p[3]) / 255.0;
        let blend = |c: u8| (f32::from(c) * a + 255.0 * (1.0 - a)).round() as u8;
        rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    let mut png = Vec::new();
    DynamicImage::ImageRgb8(rgb).write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

/// What the vision model wrote, interpreted.
#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    /// Running text, with its math as `$...$`.
    Prose(String),
    /// Just a formula: the LaTeX, without delimiters.
    Formula(String),
}

struct Segment {
    math: bool,
    body: String,
}

/// Split Markdown into text and math pieces: `$$..$$`, `\[..\]`, `\(..\)` and `$..$`.
fn segments(t: &str) -> Vec<Segment> {
    const PAIRS: [(&str, &str); 4] = [("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)"), ("$", "$")];
    let mut out = Vec::new();
    let (mut text, mut i) = (String::new(), 0);
    'scan: while i < t.len() {
        let rest = &t[i..];
        for (open, close) in PAIRS {
            if let Some(after) = rest.strip_prefix(open)
                && let Some(end) = after.find(close)
            {
                if !text.is_empty() {
                    out.push(Segment {
                        math: false,
                        body: std::mem::take(&mut text),
                    });
                }
                out.push(Segment {
                    math: true,
                    body: after[..end].trim().to_string(),
                });
                i += open.len() + end + close.len();
                continue 'scan;
            }
        }
        let ch = rest.chars().next().unwrap();
        text.push(ch);
        i += ch.len_utf8();
    }
    if !text.is_empty() {
        out.push(Segment {
            math: false,
            body: text,
        });
    }
    out
}

fn strip_fences(t: &str) -> &str {
    let t = t.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let rest = rest.split_once('\n').map_or(rest, |(_, body)| body);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

fn real_words(s: &str) -> usize {
    s.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| w.len() >= 3)
        .count()
}

/// Turn the model's Markdown into either prose or a bare formula.
///
/// Anything with a real word next to its math is prose, even a one-line exercise like
/// `Is $C_6$ bi-graceful?`: calling that a formula would throw the words away. Only output that is
/// essentially pure math (or plain text too short to be a sentence) is a formula.
pub fn interpret(raw: &str) -> Reading {
    let segs = segments(strip_fences(raw));
    let outside: String = segs
        .iter()
        .filter(|s| !s.math)
        .map(|s| s.body.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let has_math = segs.iter().any(|s| s.math);
    let words = real_words(&outside);
    if (has_math && words >= 1) || words >= 3 {
        // One line of text; every piece of math becomes an inline $...$.
        let joined: String = segs
            .iter()
            .map(|s| {
                if s.math {
                    format!("${}$", s.body)
                } else {
                    s.body.clone()
                }
            })
            .collect();
        let line = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        return Reading::Prose(prose::mark_math(&line));
    }
    let formula = segs
        .iter()
        .find(|s| s.math)
        .map_or_else(|| outside.trim().to_string(), |s| s.body.clone());
    Reading::Formula(formula)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_plan_is_ready_only_when_both_the_helper_and_the_model_are_there() {
        let dir = std::env::temp_dir().join(format!("texcopy-plan-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let fake_server = dir.join("llama-server");
        std::fs::write(&fake_server, b"x").unwrap();

        // helper present, model missing: not ready, and the model is the only item to download
        let plan = super::plan(Some(&fake_server), Some(dir.clone())).unwrap();
        assert!(!plan.is_ready());
        assert_eq!(plan.items().len(), 1);
        assert_eq!(plan.total_bytes(), super::model_download_bytes());

        // both there: ready, nothing to download
        std::fs::write(dir.join("model.gguf"), b"x").unwrap();
        std::fs::write(dir.join("mmproj.gguf"), b"x").unwrap();
        let plan = super::plan(Some(&fake_server), Some(dir.clone())).unwrap();
        assert!(plan.is_ready());
        assert!(plan.items().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    use super::{Reading, interpret, segments, strip_fences};

    #[test]
    fn prose_keeps_its_math_and_marks_plain_variables() {
        assert_eq!(
            interpret(r"5.4 Prove that if v is a cut-vertex of a graph G, then v is not a cut-vertex of the complement \(\overline{G}\) of G."),
            Reading::Prose(r"5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.".into())
        );
        // Already marked up, spaces inside the dollars, wrapped over several lines.
        assert_eq!(
            interpret(
                "Let $ G $ be a graph with $ n \\geq 3 $ vertices.\nShow that $G$ is connected."
            ),
            Reading::Prose(
                "Let $G$ be a graph with $n \\geq 3$ vertices. Show that $G$ is connected.".into()
            )
        );
    }

    #[test]
    fn display_math_inside_prose_becomes_inline() {
        assert_eq!(
            interpret(
                "Prove that for every integer n\n\n$$\\sum_{k=1}^n k = \\frac{n(n+1)}{2}$$\n\nholds."
            ),
            Reading::Prose(
                "Prove that for every integer $n$ $\\sum_{k=1}^n k = \\frac{n(n+1)}{2}$ holds."
                    .into()
            )
        );
    }

    #[test]
    fn a_bare_formula_is_a_formula_whatever_the_delimiters() {
        let want = Reading::Formula(r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}".into());
        for raw in [
            r"$$x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$$",
            r"\[ x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a} \]",
            r"\( x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a} \)",
            "```latex\n$$x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}$$\n```",
            r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        ] {
            assert_eq!(interpret(raw), want, "{raw}");
        }
    }

    #[test]
    fn a_formula_with_a_little_text_inside_it_is_still_a_formula() {
        // The words are inside the math, so there is nothing around it to keep.
        assert!(matches!(
            interpret(r"$$f(x) = 0 \text{ for } x < 1$$"),
            Reading::Formula(_)
        ));
    }

    #[test]
    fn one_line_exercises_keep_their_words() {
        // Real shapes of short textbook exercises that an earlier version turned into bare formulas,
        // dropping the words around the math.
        for raw in [
            "3.42 Let $G = 3K_{2} + K_{1}$.",
            "11.28 Determine $mr(C_{3}, C_{4})$.",
            "8.49 Is $C_{6}$ bi-graceful?",
        ] {
            assert_eq!(interpret(raw), Reading::Prose(raw.into()), "{raw}");
        }
    }

    #[test]
    fn segments_ignore_unmatched_dollars_and_fences_are_stripped() {
        let s = segments("it costs 5$ only, and x is $y$");
        assert_eq!(
            s.iter().filter(|s| s.math).count(),
            1,
            "the lone $ is text, $y$ is math"
        );
        assert_eq!(
            segments("just 5$ here").iter().filter(|s| s.math).count(),
            0
        );
        assert_eq!(strip_fences("```\nabc\n```"), "abc");
        assert_eq!(strip_fences("  plain  "), "plain");
    }
}
