//! the vision engine, OvisOCR2 served by a short lived llama-server.

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
    convert,
    fetch::{self, Download},
    helper, table,
    ui::{self, Event, Reporter},
};

const BASE_URL: &str = "https://huggingface.co/bartowski/ATH-MaaS_OvisOCR2-GGUF/resolve/main";

struct RemoteFile {
    /// name on hugging face.
    remote: &'static str,
    /// name we store it under.
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
/// bigger screenshots are shrunk to this.
const MAX_SIDE: u32 = 2000;
const MAX_TOKENS: u32 = 2048;

/// the file names the vision model is saved under.
pub(crate) fn model_file_names() -> Vec<&'static str> {
    MODEL_FILES.iter().map(|f| f.local).collect()
}

fn model_download_bytes() -> u64 {
    MODEL_FILES.iter().map(|f| f.size).sum()
}

pub struct Files {
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

/// where the model lives, an explicit dir or <cache>/ovisocr2.
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

/// download what's missing, checksum verified, with progress.
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

/// llama-server we installed, else the first on PATH, --llama-server is used as given.
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
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        // a program started from Finder doesn't get the shell's PATH so look where homebrew and distro packages put it.
        dirs.extend(
            ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
                .iter()
                .filter(|_| cfg!(unix))
                .map(PathBuf::from),
        );
        dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
    })
}

pub const INSTALL_HINT: &str = "texcopy's vision engine needs `llama-server` from llama.cpp:\n  macOS:   brew install llama.cpp\n  Windows: winget install llama.cpp   (or download a release from https://github.com/ggml-org/llama.cpp/releases)\n  Linux:   a release build from the page above, or your package manager\nThen put it on your PATH, or pass --llama-server /path/to/llama-server.";

pub struct Setup {
    pub server: PathBuf,
    pub files: Files,
}

/// the vision engine if it's set up, never asks or downloads.
pub fn ready(server_over: Option<&Path>, dir_over: Option<PathBuf>) -> Option<Setup> {
    let server = find_server(server_over)?;
    let files = files_in(&model_dir(dir_over)?)?;
    Some(Setup { server, files })
}

/// what setting up the vision engine involves, worked out without the network.
pub struct Plan {
    server: Option<PathBuf>,
    files: Option<Files>,
    dir: PathBuf,
    release: Option<&'static helper::Release>,
}

/// one thing to download.
pub struct PlanItem {
    pub what: String,
    pub size: u64,
    pub from: &'static str,
    pub to: String,
}

impl Plan {
    /// nothing left to download.
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

    /// download and install what's missing, reporting progress.
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

/// work out what the vision engine still needs.
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

/// set the vision engine up from the terminal, explain, ask once, then do it. --yes skips the ask.
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

/// <temp>/texcopy-llama-<owner pid>.pid, the helper's pid written by the texcopy that started it.
fn pidfile(owner: u32) -> PathBuf {
    std::env::temp_dir().join(format!("texcopy-llama-{owner}.pid"))
}

/// kill helpers left by a texcopy that crashed, only if the owner is gone and it really is llama-server.
pub(crate) fn reap_orphans() {
    reap_in(
        &std::env::temp_dir(),
        &process_alive,
        &is_llama_server,
        &kill_process,
    );
}

fn reap_in(
    dir: &Path,
    owner_alive: &dyn Fn(u32) -> bool,
    is_ours: &dyn Fn(u32) -> bool,
    kill: &dyn Fn(u32),
) {
    let me = std::process::id();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(owner) = name
            .strip_prefix("texcopy-llama-")
            .and_then(|r| r.strip_suffix(".pid"))
            .and_then(|r| r.parse::<u32>().ok())
        else {
            continue;
        };
        if owner == me || owner_alive(owner) {
            continue;
        }
        if let Some(helper) = fs::read_to_string(entry.path())
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
            && is_ours(helper)
        {
            kill(helper);
        }
        let _ = fs::remove_file(entry.path());
        let _ = fs::remove_file(dir.join(format!("texcopy-llama-{owner}.log")));
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(unix)]
fn is_llama_server(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("llama-server"))
}

#[cfg(unix)]
fn kill_process(pid: u32) {
    let _ = Command::new("kill")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status();
}

// windows has no ps or kill so no orphan cleanup there.
#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}
#[cfg(not(unix))]
fn is_llama_server(_pid: u32) -> bool {
    false
}
#[cfg(not(unix))]
fn kill_process(_pid: u32) {}

/// a running llama-server, stopped when dropped.
pub struct Server {
    child: Child,
    port: u16,
    log: PathBuf,
    agent: ureq::Agent,
}

impl Server {
    pub fn start(bin: &Path, files: &Files) -> Result<Self> {
        reap_orphans();
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let log = std::env::temp_dir().join(format!("texcopy-llama-{}.log", std::process::id()));
        let out = File::create(&log)?;
        let child = Command::new(bin)
            .arg("-m")
            .arg(&files.model)
            .arg("--mmproj")
            .arg(&files.mmproj)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // all layers on the GPU if there is one, one slot and no prompt cache since it grows the process from 1.4 GB to over 3 GB.
            .args(["-ngl", "99", "-c", "8192", "-np", "1", "--cache-ram", "0"])
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .spawn()
            .with_context(|| format!("starting {}", bin.display()))?;
        // leave a note of who owns this helper so a later run can clean up if we get killed.
        let _ = fs::write(pidfile(std::process::id()), child.id().to_string());
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
            // 503 while the model loads, 200 when it's ready.
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

    /// read the image, returns the markdown and whether it hit the token limit.
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
        let _ = fs::remove_file(pidfile(std::process::id()));
    }
}

/// png, shrunk if huge and flattened onto white.
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

/// what the vision model wrote, interpreted.
#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    /// running text with its math as $...$.
    Prose(String),
    /// just a formula, the LaTeX without delimiters.
    Formula(String),
}

pub struct Segment {
    pub math: bool,
    /// $$..$$ or \[..\], drawn on a line of its own.
    pub display: bool,
    pub body: String,
}

/// split markdown into text and math pieces.
pub fn segments(t: &str) -> Vec<Segment> {
    const PAIRS: [(&str, &str, bool); 4] = [
        ("$$", "$$", true),
        ("\\[", "\\]", true),
        ("\\(", "\\)", false),
        ("$", "$", false),
    ];
    let mut out = Vec::new();
    let (mut text, mut i) = (String::new(), 0);
    'scan: while i < t.len() {
        let rest = &t[i..];
        for (open, close, display) in PAIRS {
            if let Some(after) = rest.strip_prefix(open)
                && let Some(end) = closing(after, close)
                && !(open == "$" && reads_like_a_sentence(&after[..end]))
            {
                if !text.is_empty() {
                    out.push(Segment {
                        math: false,
                        display: false,
                        body: std::mem::take(&mut text),
                    });
                }
                out.push(Segment {
                    math: true,
                    display,
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
            display: false,
            body: text,
        });
    }
    out
}

/// two or more real words at the top level is a sentence between two prices, commands and braces never count.
fn reads_like_a_sentence(body: &str) -> bool {
    let (mut plain, mut depth) = (String::new(), 0usize);
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                while chars.next_if(char::is_ascii_alphabetic).is_some() {}
                plain.push(' ');
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    real_words(&plain) >= 2
}

/// where the math that opens before after closes, a $ followed by a digit is the next price.
fn closing(after: &str, close: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = after[from..].find(close) {
        let end = from + at;
        let next = after[end + close.len()..].chars().next();
        if close != "$" || !next.is_some_and(|c| c.is_ascii_digit()) {
            return Some(end);
        }
        from = end + close.len();
    }
    None
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
    s.split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 3)
        .count()
}

/// output with an HTML table, each table becomes a tabular, None if there's no usable table.
fn interpret_with_tables(raw: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = raw;
    let mut found = false;
    while let Some((start, end)) = table::find(rest) {
        let Some(latex) = table::to_latex(&rest[start..end], &convert::cell) else {
            break;
        };
        found = true;
        if !rest[..start].trim().is_empty() {
            parts.push(convert::to_latex(&rest[..start]));
        }
        parts.push(latex);
        rest = &rest[end..];
    }
    if !found {
        return None;
    }
    if !rest.trim().is_empty() {
        parts.push(convert::to_latex(rest));
    }
    Some(parts.join("\n"))
}

/// a heading, bullet or code fence at the start of a line is structure.
fn has_structure(raw: &str) -> bool {
    raw.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("```")
            || t.starts_with("# ")
            || t.starts_with("## ")
            || t.starts_with("### ")
            || ['-', '*', '•', '–']
                .iter()
                .any(|&c| t.strip_prefix(c).is_some_and(|r| r.starts_with(' ')))
    })
}

/// turn the model's markdown into prose or a bare formula, anything with a real word next to its math is prose.
pub fn interpret(raw: &str) -> Reading {
    let raw = strip_fences(raw);
    if let Some(text) = interpret_with_tables(raw) {
        return Reading::Prose(text);
    }
    let segs = segments(raw);
    let outside: String = segs
        .iter()
        .filter(|s| !s.math)
        .map(|s| s.body.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let has_math = segs.iter().any(|s| s.math);
    let words = real_words(&outside);
    // a formula picture never comes back as several lines of plain text.
    let text_lines = outside.lines().filter(|l| !l.trim().is_empty()).count();
    if (has_math && words >= 1) || words >= 3 || has_structure(raw) || text_lines >= 2 {
        return Reading::Prose(convert::to_latex(raw));
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

        // helper there and model missing, the model is the only thing to download.
        let plan = super::plan(Some(&fake_server), Some(dir.clone())).unwrap();
        assert!(!plan.is_ready());
        assert_eq!(plan.items().len(), 1);
        assert_eq!(plan.total_bytes(), super::model_download_bytes());

        // both there, ready.
        std::fs::write(dir.join("model.gguf"), b"x").unwrap();
        std::fs::write(dir.join("mmproj.gguf"), b"x").unwrap();
        let plan = super::plan(Some(&fake_server), Some(dir.clone())).unwrap();
        assert!(plan.is_ready());
        assert!(plan.items().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// what the model returned for the NYU sample document.
    const NYU_RAW: &str = "Hello World! Today I am learning LATEX. LATEX is a great program for writing math. I can write in line math such as  $a^2 + b^2 = c^2$.\n\nI can also give equations their own space:\n\n$$ \\gamma^{2}+\\theta^{2}=\\omega^{2} $$\n\n(1)\n\n1";

    #[test]
    fn a_numbered_display_equation_becomes_an_equation_environment() {
        assert_eq!(
            interpret(NYU_RAW),
            Reading::Prose(
                "Hello World! Today I am learning \\LaTeX{}. \\LaTeX{} is a great program for writing math. I can write in line math such as $a^2 + b^2 = c^2$.\n\nI can also give equations their own space:\n\\begin{equation}\n\\gamma^{2}+\\theta^{2}=\\omega^{2}\n\\end{equation}"
                    .into()
            )
        );
    }

    #[test]
    fn text_after_an_equation_starts_on_its_own_line() {
        assert_eq!(
            interpret("We know that\n\n$$E = mc^2$$\n\n(2.1)\n\nwhere $m$ is the mass."),
            Reading::Prose(
                "We know that\n\\begin{equation}\nE = mc^2\n\\end{equation}\nwhere $m$ is the mass."
                    .into()
            )
        );
        // the number written inside the maths with \tag.
        assert_eq!(
            interpret("We know that\n\n$$E = mc^2 \\tag{3}$$\n\nholds."),
            Reading::Prose(
                "We know that\n\\begin{equation}\nE = mc^2\n\\end{equation}\nholds.".into()
            )
        );
    }

    #[test]
    fn a_display_after_a_colon_gets_brackets_but_one_mid_sentence_stays_inline() {
        assert_eq!(
            interpret(
                "Recall the identity:\n\n$$\\sum_{k=1}^n k = \\frac{n(n+1)}{2}$$\n\nIt is easy."
            ),
            Reading::Prose(
                "Recall the identity:\n\\[\n\\sum_{k=1}^n k = \\frac{n(n+1)}{2}\n\\]\nIt is easy."
                    .into()
            )
        );
        // display_math_inside_prose_becomes_inline covers the mid sentence case.
    }

    #[test]
    fn a_page_number_is_only_dropped_after_a_display_equation() {
        // no display, so a trailing number is content and stays its own paragraph.
        assert_eq!(
            interpret("Compute the sum of the first $n$ terms.\n\n42"),
            Reading::Prose("Compute the sum of the first $n$ terms.\n\n42".into())
        );
        // an equation reference in running text stays as text.
        assert!(
            matches!(interpret("See the equation (1) above for $x$."), Reading::Prose(p) if p.contains("(1)"))
        );
    }

    #[test]
    fn an_html_table_becomes_a_tabular_and_the_heading_above_it_stays_text() {
        let raw = "Execution-time comparison\n\n<table border=1><tr><td>n</td><td>Recursive time (ms)</td><td>DP time (ms)</td></tr><tr><td>5</td><td>0.000037</td><td>0.000022</td></tr><tr><td>23</td><td>13.314562</td><td>0.000154</td></tr></table>";
        assert_eq!(
            interpret(raw),
            Reading::Prose(
                "Execution-time comparison\n\\begin{center}\n\\begin{tabular}{rrr}\n\\hline\n$n$ & Recursive time (ms) & DP time (ms) \\\\\n\\hline\n5 & 0.000037 & 0.000022 \\\\\n23 & 13.314562 & 0.000154 \\\\\n\\hline\n\\end{tabular}\n\\end{center}"
                    .into()
            )
        );
    }

    #[test]
    fn math_with_environment_names_or_text_is_never_mistaken_for_a_sentence() {
        // real output for a cases image, cases shows up twice in the body.
        assert!(matches!(
            interpret(r"$\begin{cases} x+y=2 \\ x-y=0 \end{cases}$"),
            Reading::Formula(f) if f.starts_with(r"\begin{cases}")
        ));
        assert!(matches!(
            interpret(r"$\text{for all } x \in \mathbb{R}, \quad f(x) \geq 0$"),
            Reading::Formula(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_whose_owner_died_is_killed_but_only_if_it_really_is_the_helper() {
        use std::{cell::RefCell, process::Command};
        let dir = std::env::temp_dir().join(format!("texcopy-reap-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        // a real process that already exited, its pid is a dead owner.
        let mut gone = Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let (dead_owner, live_owner) = (gone.id(), std::process::id() + 1);

        let write = |owner: u32, helper: u32| {
            std::fs::write(
                dir.join(format!("texcopy-llama-{owner}.pid")),
                helper.to_string(),
            )
            .unwrap();
        };
        write(dead_owner, 4242); // owner dead and the helper is the real thing, kill it.
        write(live_owner, 4343); // owner still running, leave it.
        std::fs::write(dir.join("unrelated.pid"), "1").unwrap();

        let killed = RefCell::new(Vec::new());
        super::reap_in(&dir, &|p| p == live_owner, &|h| h == 4242, &|h| {
            killed.borrow_mut().push(h)
        });
        assert_eq!(*killed.borrow(), vec![4242]);
        assert!(
            !dir.join(format!("texcopy-llama-{dead_owner}.pid")).exists(),
            "stale note removed"
        );
        assert!(
            dir.join(format!("texcopy-llama-{live_owner}.pid")).exists(),
            "live owner's note kept"
        );
        assert!(dir.join("unrelated.pid").exists());

        // a pid reused by another program never gets killed.
        write(dead_owner, 9999);
        killed.borrow_mut().clear();
        super::reap_in(&dir, &|_| false, &|_| false, &|h| {
            killed.borrow_mut().push(h)
        });
        assert!(killed.borrow().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    use super::{Reading, interpret, segments, strip_fences};

    #[test]
    fn prose_keeps_its_math_and_marks_plain_variables() {
        assert_eq!(
            interpret(r"5.4 Prove that if v is a cut-vertex of a graph G, then v is not a cut-vertex of the complement \(\overline{G}\) of G."),
            Reading::Prose(r"5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.".into())
        );
        // already marked up, spaces inside the dollars and wrapped over lines.
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
    fn display_math_inside_prose_is_a_display_equation() {
        // the picture shows it centred on its own line so rebuild it as a display.
        assert_eq!(
            interpret(
                "Prove that for every integer n\n\n$$\\sum_{k=1}^n k = \\frac{n(n+1)}{2}$$\n\nholds."
            ),
            Reading::Prose(
                "Prove that for every integer $n$\n\\[\n\\sum_{k=1}^n k = \\frac{n(n+1)}{2}\n\\]\nholds."
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
        // the words are inside the math so nothing around it to keep.
        assert!(matches!(
            interpret(r"$$f(x) = 0 \text{ for } x < 1$$"),
            Reading::Formula(_)
        ));
    }

    #[test]
    fn one_line_exercises_keep_their_words() {
        // real shapes of short exercises that used to become bare formulas and lose their words.
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
