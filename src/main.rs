mod fetch;
mod model;
mod spec;
mod tidy;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use image::DynamicImage;

#[derive(Clone, Copy, ValueEnum)]
enum Wrap {
    /// Raw LaTeX, no delimiters
    None,
    /// $...$
    Inline,
    /// \[ ... \]
    Display,
}

/// Select something LaTeX-rendered on screen and get its LaTeX source on your clipboard.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Image file to read. Omit to drag-select a screen region (macOS).
    image: Option<PathBuf>,

    /// Read the image from the clipboard instead
    #[arg(long, conflicts_with = "image")]
    clipboard: bool,

    /// Delimiters to put around the result
    #[arg(short, long, value_enum, default_value_t = Wrap::None)]
    wrap: Wrap,

    /// Print the LaTeX but don't touch the clipboard
    #[arg(long)]
    no_copy: bool,

    /// Directory containing the model files and optional spec.json (default: ./models, else the cache dir)
    #[arg(long, env = "TEXCOPY_MODEL_DIR")]
    model_dir: Option<PathBuf>,

    /// Skip cleanup and print the model's token stream as-is
    #[arg(long)]
    raw: bool,

    /// Upper bound on generated tokens
    #[arg(long, default_value_t = 512)]
    max_tokens: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Grab the image first so a cancelled selection costs nothing.
    let Some(img) = load_image(&cli)? else {
        eprintln!("Cancelled.");
        return Ok(());
    };

    let (dir, spec) = fetch::locate(cli.model_dir)?;
    let mut rec = model::Recognizer::load(&dir, spec)?;
    let latex = rec.recognize(&img, cli.max_tokens)?;
    let latex = if cli.raw { latex } else { tidy::tidy(&latex) };

    let out = match cli.wrap {
        Wrap::None => latex,
        Wrap::Inline => format!("${latex}$"),
        Wrap::Display => format!("\\[ {latex} \\]"),
    };
    println!("{out}");
    if !cli.no_copy {
        arboard::Clipboard::new()?.set_text(out).context("writing to clipboard")?;
        eprintln!("Copied to clipboard.");
    }
    Ok(())
}

/// `None` means the user cancelled the screen selection.
fn load_image(cli: &Cli) -> Result<Option<DynamicImage>> {
    if let Some(path) = &cli.image {
        return Ok(Some(image::open(path).with_context(|| format!("opening {}", path.display()))?));
    }
    if cli.clipboard {
        let img = arboard::Clipboard::new()?.get_image().context("no image on the clipboard")?;
        let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
            .context("malformed clipboard image")?;
        return Ok(Some(buf.into()));
    }
    capture_region()
}

#[cfg(target_os = "macos")]
fn capture_region() -> Result<Option<DynamicImage>> {
    use std::process::Command;
    let path = std::env::temp_dir().join(format!("texcopy-{}.png", std::process::id()));
    // -i: interactive crosshair selection, -x: no shutter sound. Esc cancels (no file written).
    let status = Command::new("screencapture").args(["-i", "-x"]).arg(&path).status()?;
    if !status.success() || !path.exists() {
        return Ok(None);
    }
    let img = image::open(&path);
    let _ = std::fs::remove_file(&path);
    Ok(Some(img?))
}

#[cfg(not(target_os = "macos"))]
fn capture_region() -> Result<Option<DynamicImage>> {
    anyhow::bail!("screen capture is only built in on macOS; pass an image file or use --clipboard")
}
