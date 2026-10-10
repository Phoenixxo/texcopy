//! Web-style pictures: KaTeX drawn in headless Chrome, light and dark, like a page on screen.
//!
//! KaTeX (MIT) is installed on first use into `tools/.cache/katex` (git-ignored) with npm. Needs
//! `node`, `npm` and Chrome (set CHROME to point at it if it is somewhere unusual).

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::{
    manifest::{Case, Grading},
    render::{Scratch, run_timed, trim_border},
    rng::Rng,
    util::{par_map, tools_dir},
};

const NODE_SCRIPT: &str = r#"
const fs = require("fs");
const katex = require(process.argv[2] + "/node_modules/katex");
const cases = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
const outdir = process.argv[4];
const css = "file://" + process.argv[2] + "/node_modules/katex/dist/katex.min.css";
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const failed = [];
for (const c of cases) {
  try {
    let body = "";
    for (const p of c.parts) {
      body += p.t === "text" ? esc(p.v)
        : katex.renderToString(p.v, { displayMode: !!p.display, throwOnError: true, strict: "ignore" });
    }
    const html = `<!doctype html><meta charset="utf-8"><link rel="stylesheet" href="${css}">
<body style="margin:0;padding:28px;width:${c.width}px;background:${c.bg};color:${c.fg};font:${c.font_px}px/1.5 ${c.family}">${body}</body>`;
    fs.writeFileSync(outdir + "/" + c.name + ".html", html);
  } catch (e) { failed.push(c.name); }
}
console.log(JSON.stringify(failed));
"#;

const FAMILIES: [&str; 3] = [
    r#""Times New Roman", serif"#,
    "Georgia, serif",
    "Helvetica, Arial, sans-serif",
];

#[derive(Serialize)]
struct Part {
    t: &'static str,
    v: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display: Option<bool>,
}

#[derive(Serialize)]
struct Spec {
    name: String,
    parts: Vec<Part>,
    bg: &'static str,
    fg: &'static str,
    font_px: u32,
    width: u32,
    family: &'static str,
}

fn katex_dir() -> PathBuf {
    tools_dir().join(".cache").join("katex")
}

pub fn find_chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CHROME") {
        return Some(PathBuf::from(path));
    }
    let mac = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    if mac.exists() {
        return Some(mac.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome.exe",
    ]
    .iter()
    .flat_map(|name| std::env::split_paths(&path).map(move |d| d.join(name)))
    .find(|p| p.is_file())
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

/// Install KaTeX once. Returns the folder that holds `node_modules/katex`.
fn ensure_katex() -> Result<PathBuf> {
    let dir = katex_dir();
    if dir.join("node_modules").join("katex").is_dir() {
        return Ok(dir);
    }
    for tool in ["node", "npm"] {
        if !have(tool) {
            bail!("web cases need `{tool}` (install Node.js), or leave them out with --kinds");
        }
    }
    eprintln!("installing KaTeX into tools/.cache/katex (once) ...");
    fs::create_dir_all(&dir)?;
    let out = Command::new("npm")
        .args(["install", "--silent", "--no-audit", "--no-fund", "--prefix"])
        .arg(&dir)
        .arg("katex")
        .output()?;
    if !out.status.success() {
        bail!(
            "npm install katex failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(dir)
}

/// Split LaTeX text with `$...$` into text and math pieces. A formula is one display piece.
fn parts_of(truth: &str, kind: Grading) -> Vec<Part> {
    if kind == Grading::Formula {
        return vec![Part {
            t: "math",
            v: truth.to_string(),
            display: Some(true),
        }];
    }
    let mut parts = Vec::new();
    let mut rest = truth;
    while !rest.is_empty() {
        let Some(open) = rest.find('$') else {
            parts.push(Part {
                t: "text",
                v: rest.to_string(),
                display: None,
            });
            break;
        };
        let Some(len) = rest[open + 1..].find('$') else {
            parts.push(Part {
                t: "text",
                v: rest.to_string(),
                display: None,
            });
            break;
        };
        if open > 0 {
            parts.push(Part {
                t: "text",
                v: rest[..open].to_string(),
                display: None,
            });
        }
        parts.push(Part {
            t: "math",
            v: rest[open + 1..open + 1 + len].to_string(),
            display: Some(false),
        });
        rest = &rest[open + len + 2..];
    }
    parts
}

fn screenshot(chrome: &Path, html: &Path, png: &Path, width: u32) -> Result<bool> {
    let profile = Scratch::new()?;
    run_timed(
        Command::new(chrome)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--force-device-scale-factor=2",
            ])
            .arg(format!("--user-data-dir={}", profile.0.display()))
            .args([
                "--virtual-time-budget=3000",
                &format!("--window-size={},700", width + 80),
            ])
            .arg(format!("--screenshot={}", png.display()))
            .arg(format!("file://{}", html.display())),
        60,
    )?;
    let Ok(img) = image::open(png) else {
        return Ok(false);
    };
    trim_border(&img.to_rgb8(), 20).save(png)?;
    Ok(true)
}

/// Render the queued web cases into `out_dir`. Returns the names that worked.
pub fn render_katex_cases(
    cases: &mut [Case],
    out_dir: &Path,
    jobs: usize,
) -> Result<HashSet<String>> {
    let katex = ensure_katex()?;
    let chrome = find_chrome().context("web cases need Google Chrome (set CHROME to its path)")?;
    let work = Scratch::new()?;
    let mut specs = Vec::new();
    for c in cases.iter_mut() {
        let mut rng = Rng::from_text(&format!("{}-web", c.seed));
        let (bg, fg) = if c.theme.as_deref() == Some("dark") {
            ("#1e1f22", "#dcddde")
        } else {
            ("#ffffff", "#111111")
        };
        let family = *rng.pick(&FAMILIES);
        c.family = Some(family.to_string());
        specs.push(Spec {
            name: c.name.clone(),
            parts: parts_of(&c.truth, c.kind),
            bg,
            fg,
            font_px: c.font_px.unwrap_or(26),
            width: *rng.pick(&[560, 720, 860]),
            family,
        });
    }
    let spec_file = work.0.join("cases.json");
    fs::write(&spec_file, serde_json::to_string(&specs)?)?;
    let script = work.0.join("render.js");
    fs::write(&script, NODE_SCRIPT)?;
    let out = Command::new("node")
        .arg(&script)
        .arg(&katex)
        .arg(&spec_file)
        .arg(&work.0)
        .output()?;
    if !out.status.success() {
        bail!("KaTeX failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let failed: HashSet<String> = serde_json::from_str(stdout.lines().last().unwrap_or("[]"))?;
    let todo: Vec<(&Spec, &Case)> = specs
        .iter()
        .zip(cases.iter())
        .filter(|(s, _)| !failed.contains(&s.name))
        .collect();
    let results = par_map(todo.clone(), jobs, |(s, c)| {
        let html = work.0.join(format!("{}.html", s.name));
        let ok = screenshot(&chrome, &html, &out_dir.join(&c.image), s.width).unwrap_or(false);
        ok.then(|| s.name.clone())
    });
    let ok: HashSet<String> = results.into_iter().flatten().collect();
    eprintln!(
        "  web: {} rendered, {} rejected by KaTeX, {} screenshots failed",
        ok.len(),
        failed.len(),
        todo.len() - ok.len()
    );
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_splits_into_text_and_math_and_a_formula_is_one_display_piece() {
        let p = parts_of("Let $x$ be $n+1$ here.", Grading::Document);
        let shape: Vec<(&str, &str)> = p.iter().map(|p| (p.t, p.v.as_str())).collect();
        assert_eq!(
            shape,
            [
                ("text", "Let "),
                ("math", "x"),
                ("text", " be "),
                ("math", "n+1"),
                ("text", " here.")
            ]
        );
        let f = parts_of(r"\frac{a}{b}", Grading::Formula);
        assert_eq!((f.len(), f[0].display), (1, Some(true)));
    }
}
