mod capture;
mod clipboard;
mod fetch;
mod model;
mod spec;
mod tidy;

use std::{io::Read, path::PathBuf};

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
    /// Image file to read. With no input option, drag-select a screen region (macOS only for now).
    image: Option<PathBuf>,

    /// Read the image from the clipboard instead
    #[arg(long, conflicts_with_all = ["image", "screen"])]
    clipboard: bool,

    /// Capture a whole monitor (the primary one unless --monitor is given)
    #[arg(long, conflicts_with = "image")]
    screen: bool,

    /// Monitor index for --screen (0 is the first monitor the OS lists)
    #[arg(long, requires = "screen")]
    monitor: Option<usize>,

    /// Crop --screen to x,y,w,h in screenshot pixels
    #[arg(long, requires = "screen", value_name = "x,y,w,h")]
    region: Option<String>,

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

    /// Override how the image is fitted to the model's input (default: the model's own setting)
    #[arg(long, value_enum)]
    fit: Option<spec::Fit>,

    /// Upper bound on generated tokens
    #[arg(long, default_value_t = 512)]
    max_tokens: usize,

    /// Internal: serve stdin on the clipboard until overwritten (Linux)
    #[arg(long, hide = true)]
    hold_clipboard: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.hold_clipboard {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        return clipboard::hold(&text);
    }

    // Grab the image first so a cancelled selection costs nothing.
    let Some(img) = load_image(&cli)? else {
        eprintln!("Cancelled.");
        return Ok(());
    };

    let (dir, mut spec) = fetch::locate(cli.model_dir)?;
    if let Some(fit) = cli.fit {
        spec.fit = fit;
    }
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
        clipboard::copy_text(&out)?;
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
        return clipboard::read_image().map(Some);
    }
    if cli.screen {
        let shot = capture::screenshot(cli.monitor)?;
        return match &cli.region {
            Some(r) => capture::crop(&shot, r).map(Some),
            None => Ok(Some(shot)),
        };
    }
    capture::interactive()
}
