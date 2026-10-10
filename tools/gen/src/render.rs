//! Turning LaTeX into a picture with the real tools (pdflatex or xelatex, then pdftoppm), and the
//! image clean-up and degradation that make it look like a screenshot.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use image::{
    DynamicImage, Rgb, RgbImage,
    codecs::jpeg::JpegEncoder,
    imageops::{self, FilterType},
};

use crate::{manifest::Grading, rng::Rng};

/// A way of drawing LaTeX: which program, and what to load right after `\documentclass`.
pub struct Renderer {
    pub name: &'static str,
    pub engine: &'static str,
    pub early: &'static str,
}

// No Times (mathptmx) here: with this TeX install `\leq`, `\neq`, `\in`, `\cdot` and friends need the missing
// `rsfs` font, so nearly every formula fails. STIX Two (below) is the Times-like face instead.
pub const RENDERERS: [Renderer; 4] = [
    Renderer {
        name: "latex-cm",
        engine: "pdflatex",
        early: "",
    },
    Renderer {
        name: "latex-lmodern",
        engine: "pdflatex",
        early: r"\usepackage{lmodern}",
    },
    Renderer {
        name: "xelatex-stix",
        engine: "xelatex",
        early: r"\usepackage{unicode-math}\setmainfont{STIX Two Text}\setmathfont{STIX Two Math}",
    },
    Renderer {
        name: "xelatex-lm",
        engine: "xelatex",
        early: r"\usepackage{unicode-math}\setmathfont{latinmodern-math.otf}",
    },
];

pub fn renderer(name: &str) -> &'static Renderer {
    RENDERERS
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no renderer {name}"))
}

/// The whole .tex file for a formula (shown as display math) or a document body (in a minipage).
pub fn latex_source(
    body: &str,
    kind: Grading,
    r: &Renderer,
    width_cm: u32,
    pt: u32,
    preamble: &str,
) -> String {
    let packages = if r.engine == "xelatex" {
        // unicode-math replaces amssymb, and amsmath has to come first
        format!("\\usepackage{{amsmath,amsthm,array,booktabs}}\n{}", r.early)
    } else {
        format!(
            "{}\n\\usepackage{{amsmath,amssymb,array,amsthm,booktabs}}",
            r.early
        )
    };
    let shown = match kind {
        Grading::Formula => format!("\\[ {body} \\]"),
        Grading::Document => format!("\\begin{{minipage}}{{{width_cm}cm}}{body}\\end{{minipage}}"),
    };
    format!(
        "\\documentclass[{pt}pt]{{article}}\n{packages}\n{preamble}\n\\pagestyle{{empty}}\\begin{{document}}\\noindent {shown}\n\\end{{document}}\n"
    )
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh empty folder under the system temp folder. Removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new() -> Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("texcopy-gen-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

/// Run a command, giving up after `secs`. Returns whether it finished in time.
pub fn run_timed(cmd: &mut Command, secs: u64) -> Result<bool> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        if start.elapsed() > Duration::from_secs(secs) {
            child.kill().ok();
            child.wait().ok();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// How a render ended: a picture was written, or why not.
pub enum Rendered {
    Done,
    Failed(String),
}

impl Rendered {
    pub fn is_done(&self) -> bool {
        matches!(self, Rendered::Done)
    }
}

/// The first line of a LaTeX log that starts with `!` (the error), for saying why a render failed.
fn first_error(log: &Path) -> String {
    fs::read_to_string(log)
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with('!'))
                .map(|l| l.chars().take(90).collect())
        })
        .unwrap_or_else(|| "no PDF was produced".to_string())
}

/// Compile and rasterise. Says whether a usable (not blank) picture was written to `out`.
#[allow(clippy::too_many_arguments)]
pub fn render_latex(
    body: &str,
    kind: Grading,
    renderer_name: &str,
    dpi: u32,
    out: &Path,
    width_cm: u32,
    pt: u32,
    preamble: &str,
    timeout_secs: u64,
) -> Result<Rendered> {
    let r = renderer(renderer_name);
    let scratch = Scratch::new()?;
    let dir = &scratch.0;
    fs::write(
        dir.join("c.tex"),
        latex_source(body, kind, r, width_cm, pt, preamble),
    )?;
    let ok = run_timed(
        Command::new(r.engine)
            .args(["-interaction=nonstopmode", "-halt-on-error", "c.tex"])
            .current_dir(dir),
        timeout_secs,
    )
    .with_context(|| format!("running {} (is it installed?)", r.engine))?;
    if !ok {
        return Ok(Rendered::Failed(format!(
            "{} took longer than {timeout_secs}s",
            r.engine
        )));
    }
    if !dir.join("c.pdf").exists() {
        return Ok(Rendered::Failed(first_error(&dir.join("c.log"))));
    }
    Command::new("pdftoppm")
        .args(["-r", &dpi.to_string(), "-png", "-singlefile", "c.pdf", "c"])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running pdftoppm (is poppler installed?)")?;
    let Ok(img) = image::open(dir.join("c.png")) else {
        return Ok(Rendered::Failed("pdftoppm made no picture".into()));
    };
    let img = trim_border(&img.to_rgb8(), 12);
    if is_blank(&img) {
        return Ok(Rendered::Failed("the picture came out blank".into()));
    }
    img.save(out)?;
    Ok(Rendered::Done)
}

/// Crop to the ink and put a margin of the background colour around it.
pub fn trim_border(img: &RgbImage, border: u32) -> RgbImage {
    let bg = *img.get_pixel(0, 0);
    let differs = |p: &Rgb<u8>| {
        p.0.iter()
            .zip(bg.0)
            .any(|(a, b)| (i16::from(*a) - i16::from(b)).abs() > 12)
    };
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if differs(p) {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 == u32::MAX {
        return img.clone();
    }
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut out = RgbImage::from_pixel(w + 2 * border, h + 2 * border, bg);
    imageops::replace(
        &mut out,
        &imageops::crop_imm(img, x0, y0, w, h).to_image(),
        i64::from(border),
        i64::from(border),
    );
    out
}

/// Nothing drawn: all background, or the whole thing black.
pub fn is_blank(img: &RgbImage) -> bool {
    let n = u64::from(img.width()) * u64::from(img.height());
    let sum: u64 = img
        .pixels()
        .map(|p| (u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2])) / 3)
        .sum();
    let mean = sum as f64 / n.max(1) as f64 / 255.0;
    mean > 0.9995 || mean < 0.0005
}

/// Make the picture look like an imperfect screenshot, in place. Returns what was done.
pub fn degrade(path: &Path, rng: &mut Rng) -> Result<&'static str> {
    let name = *rng.pick(&["shrink", "blur", "noise", "jpeg"]);
    let img = image::open(path)?.to_rgb8();
    let out: RgbImage = match name {
        "shrink" => {
            let pct = *rng.pick(&[50u32, 60, 75]);
            let (w, h) = (
                (img.width() * pct / 100).max(8),
                (img.height() * pct / 100).max(8),
            );
            imageops::resize(&img, w, h, FilterType::Lanczos3)
        }
        "blur" => imageops::blur(&img, 1.0),
        "noise" => {
            let mut noisy = img.clone();
            for p in noisy.pixels_mut() {
                // Box-Muller: a bell-curve shaped nudge to each pixel
                let (u1, u2) = (rng.float().max(1e-12), rng.float());
                let gauss = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
                let delta = (gauss * 14.0) as i16;
                for c in &mut p.0 {
                    *c = (i16::from(*c) + delta).clamp(0, 255) as u8;
                }
            }
            noisy
        }
        _ => {
            let quality = *rng.pick(&[25u8, 40, 60]);
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut bytes, quality)
                .encode_image(&DynamicImage::ImageRgb8(img))?;
            image::load_from_memory(&bytes)?.to_rgb8()
        }
    };
    out.save(path)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_leaves_a_margin_of_the_background_around_the_ink() {
        let mut img = RgbImage::from_pixel(100, 60, Rgb([255, 255, 255]));
        for x in 40..60 {
            for y in 20..30 {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        let t = trim_border(&img, 12);
        assert_eq!((t.width(), t.height()), (20 + 24, 10 + 24));
        assert_eq!(t.get_pixel(0, 0), &Rgb([255, 255, 255]));
        assert_eq!(t.get_pixel(12, 12), &Rgb([0, 0, 0]));
    }

    #[test]
    fn trimming_works_on_a_dark_background_too() {
        let mut img = RgbImage::from_pixel(50, 50, Rgb([30, 31, 34]));
        img.put_pixel(25, 25, Rgb([220, 221, 222]));
        let t = trim_border(&img, 5);
        assert_eq!((t.width(), t.height()), (11, 11));
        assert_eq!(t.get_pixel(0, 0), &Rgb([30, 31, 34]));
    }

    #[test]
    fn blank_pictures_are_detected() {
        assert!(is_blank(&RgbImage::from_pixel(
            10,
            10,
            Rgb([255, 255, 255])
        )));
        let mut img = RgbImage::from_pixel(100, 100, Rgb([255, 255, 255]));
        for x in 0..50 {
            img.put_pixel(x, 0, Rgb([0, 0, 0]));
        }
        assert!(!is_blank(&img));
    }

    #[test]
    fn the_latex_source_wraps_a_formula_in_display_math_and_a_document_in_a_minipage() {
        let f = latex_source(
            "x^2",
            Grading::Formula,
            renderer("latex-lmodern"),
            12,
            12,
            "",
        );
        assert!(f.contains(r"\[ x^2 \]") && f.contains(r"\usepackage{lmodern}"));
        let d = latex_source(
            "Hello.",
            Grading::Document,
            renderer("latex-cm"),
            10,
            11,
            r"\newcommand{\R}{x}",
        );
        assert!(
            d.contains(r"\begin{minipage}{10cm}Hello.\end{minipage}")
                && d.contains(r"\newcommand{\R}{x}")
        );
        let x = latex_source("a", Grading::Formula, renderer("xelatex-stix"), 12, 12, "");
        assert!(x.contains("unicode-math") && !x.contains("amssymb"));
    }
}
