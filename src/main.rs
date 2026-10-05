mod capture;
mod clipboard;
mod fetch;
mod model;
mod prose;
mod spec;
mod tidy;

use std::{io::Read, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use image::DynamicImage;
use serde::Serialize;

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

    /// Print machine-readable JSON (latex, raw, stop reason, per-token confidence) and don't copy
    #[arg(long)]
    json: bool,

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
    let result = rec.recognize(&img, cli.max_tokens)?;
    let (latex, kind) = if cli.raw {
        (result.text.clone(), tidy::Kind::Math)
    } else {
        tidy::tidy_with_kind(&result.text)
    };

    if cli.json {
        let out = JsonOut {
            latex: &latex,
            kind,
            raw: &result.text,
            stop: result.stop,
            min_prob: result.min_prob(),
            mean_logprob: result.mean_logprob(),
            tokens: &result.tokens,
        };
        println!("{}", serde_json::to_string(&out)?);
        return Ok(());
    }
    match result.stop {
        model::Stop::Looped => anyhow::bail!(
            "couldn't read this: the model got stuck repeating itself. Try selecting a smaller or cleaner region."
        ),
        model::Stop::MaxTokens => eprintln!(
            "warning: stopped at --max-tokens ({}); the output is probably cut off",
            cli.max_tokens
        ),
        model::Stop::Eos => {}
    }
    let unsure = result.uncertain();
    if !unsure.is_empty() {
        eprintln!(
            "warning: unsure about {} token(s), worth checking:",
            unsure.len()
        );
        for &i in unsure.iter().take(5) {
            // Show a little context before the token, which is marked [like this].
            let before: String = result.tokens[i.saturating_sub(4)..i]
                .iter()
                .map(|t| t.text.trim())
                .collect();
            let t = &result.tokens[i];
            eprintln!(
                "  ...{before}[{}]  ({:.0}% sure)",
                t.text.trim(),
                t.prob * 100.0
            );
        }
    }

    if kind == tidy::Kind::Prose && prose::has_run_together_words(&latex) {
        eprintln!(
            "warning: some words look run together (the model lost the spaces). Try selecting one line at a time."
        );
    }
    if latex.contains("\\nu") {
        eprintln!(
            "note: the model read a \\nu. In an italic font, v and \u{3bd} look nearly identical, so check which one it should be."
        );
    }

    // A sentence already carries its own $...$ marks, so --wrap only applies to formulas.
    let out = match (cli.wrap, kind) {
        (Wrap::None, _) | (_, tidy::Kind::Prose) => latex,
        (Wrap::Inline, _) => format!("${latex}$"),
        (Wrap::Display, _) => format!("\\[ {latex} \\]"),
    };
    println!("{out}");
    if !cli.no_copy {
        clipboard::copy_text(&out)?;
        eprintln!("Copied to clipboard.");
    }
    Ok(())
}

#[derive(Serialize)]
struct JsonOut<'a> {
    latex: &'a str,
    kind: tidy::Kind,
    raw: &'a str,
    stop: model::Stop,
    min_prob: f32,
    mean_logprob: f32,
    tokens: &'a [model::Token],
}

/// `None` means the user cancelled the screen selection.
fn load_image(cli: &Cli) -> Result<Option<DynamicImage>> {
    if let Some(path) = &cli.image {
        return Ok(Some(
            image::open(path).with_context(|| format!("opening {}", path.display()))?,
        ));
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
