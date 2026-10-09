mod capture;
mod clipboard;
mod fetch;
mod helper;
mod model;
mod prose;
mod spec;
mod tidy;
mod ui;
mod vlm;

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

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Engine {
    /// The vision engine when it is set up, otherwise the small model
    Auto,
    /// Only the small formula model (fast, 118 MB)
    Small,
    /// Only the vision engine (OvisOCR2 via llama.cpp, about 1 GB)
    Vlm,
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

    /// Which engine reads the image. auto: the vision engine if it is set up (best for sentences
    /// and long equations), else the small formula model. vlm downloads its model on first use.
    #[arg(long, value_enum, default_value_t = Engine::Auto, env = "TEXCOPY_ENGINE")]
    engine: Engine,

    /// Set up the vision engine (the llama.cpp helper and the model), asking first, then exit
    #[arg(long)]
    setup: bool,

    /// Answer yes to setup questions instead of asking
    #[arg(short, long)]
    yes: bool,

    /// Directory with the vision model files (model.gguf and mmproj.gguf)
    #[arg(long, env = "TEXCOPY_VLM_DIR")]
    vlm_dir: Option<PathBuf>,

    /// Path to llama-server from llama.cpp (default: the one on PATH)
    #[arg(long, env = "TEXCOPY_LLAMA_SERVER")]
    llama_server: Option<PathBuf>,

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

    if cli.setup {
        vlm::ensure(cli.llama_server.as_deref(), cli.vlm_dir.clone(), cli.yes)?;
        eprintln!("Try it:  texcopy --engine vlm some-image.png");
        return Ok(());
    }

    // Grab the image first so a cancelled selection costs nothing.
    let Some(img) = load_image(&cli)? else {
        eprintln!("Cancelled.");
        return Ok(());
    };

    let outcome = read(&cli, &img)?;

    if cli.json {
        let out = JsonOut {
            latex: &outcome.latex,
            kind: outcome.kind,
            raw: &outcome.raw,
            engine: outcome.engine,
            stop: outcome.stop,
            min_prob: outcome.min_prob,
            mean_logprob: outcome.mean_logprob,
            tokens: &outcome.tokens,
        };
        println!("{}", serde_json::to_string(&out)?);
        return Ok(());
    }
    match outcome.stop {
        model::Stop::Looped => anyhow::bail!(
            "couldn't read this: the model got stuck repeating itself. Try selecting a smaller or cleaner region."
        ),
        model::Stop::MaxTokens => {
            eprintln!("warning: the output is probably cut off (token limit reached)")
        }
        model::Stop::Eos => {}
    }
    let unsure: Vec<usize> = (0..outcome.tokens.len())
        .filter(|&i| outcome.tokens[i].prob < model::LOW_CONFIDENCE)
        .collect();
    if !unsure.is_empty() {
        eprintln!(
            "warning: unsure about {} token(s), worth checking:",
            unsure.len()
        );
        for &i in unsure.iter().take(5) {
            // Show a little context before the token, which is marked [like this].
            let before: String = outcome.tokens[i.saturating_sub(4)..i]
                .iter()
                .map(|t| t.text.trim())
                .collect();
            let t = &outcome.tokens[i];
            eprintln!(
                "  ...{before}[{}]  ({:.0}% sure)",
                t.text.trim(),
                t.prob * 100.0
            );
        }
    }

    let (latex, kind) = (outcome.latex, outcome.kind);
    if kind == tidy::Kind::Prose && prose::has_run_together_words(&latex) {
        eprintln!(
            "warning: some words look run together (the model lost the spaces). Try selecting one line at a time."
        );
    }
    if outcome.engine == "small" && latex.contains("\\nu") {
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

/// What an engine read, in one shape.
struct Outcome {
    latex: String,
    kind: tidy::Kind,
    raw: String,
    engine: &'static str,
    stop: model::Stop,
    min_prob: Option<f32>,
    mean_logprob: Option<f32>,
    tokens: Vec<model::Token>,
}

/// Choose and run the engines. The vision engine goes first: if it finds text, that is the answer.
/// If it finds only a formula, the small model is preferred (it is better on isolated formulas),
/// with the vision engine's formula as the fallback.
fn read(cli: &Cli, img: &DynamicImage) -> Result<Outcome> {
    let vision = match cli.engine {
        Engine::Small => None,
        Engine::Vlm => run_vlm(cli, img, true)?,
        Engine::Auto => match run_vlm(cli, img, false) {
            Ok(None) => {
                if !cli.json {
                    eprintln!(
                        "tip: run `texcopy --setup` once to enable the vision engine, which reads whole paragraphs and long equations far better (a one-time download, it asks first)"
                    );
                }
                None
            }
            Ok(some) => some,
            Err(e) => {
                eprintln!("warning: the vision engine failed ({e:#}); using the small model");
                None
            }
        },
    };
    let Some((raw, truncated)) = vision else {
        return read_small(cli, img);
    };
    let stop = if truncated {
        model::Stop::MaxTokens
    } else {
        model::Stop::Eos
    };
    let from_vision = |latex: String, kind| Outcome {
        latex: if cli.raw { raw.clone() } else { latex },
        kind,
        raw: raw.clone(),
        engine: "vlm",
        stop,
        min_prob: None,
        mean_logprob: None,
        tokens: Vec::new(),
    };
    match vlm::interpret(&raw) {
        vlm::Reading::Prose(text) => Ok(from_vision(text, tidy::Kind::Prose)),
        vlm::Reading::Formula(f) if cli.engine == Engine::Vlm => {
            Ok(from_vision(f, tidy::Kind::Math))
        }
        vlm::Reading::Formula(f) => {
            let small = read_small(cli, img)?;
            // The small model giving up is the one case where the vision engine's formula is better.
            Ok(if small.stop == model::Stop::Looped {
                from_vision(f, tidy::Kind::Math)
            } else {
                small
            })
        }
    }
}

/// Run the vision engine. `Ok(None)` means it isn't set up, which is only allowed when `required` is
/// false; when it is required, a missing piece is set up first (asking the user).
fn run_vlm(cli: &Cli, img: &DynamicImage, required: bool) -> Result<Option<(String, bool)>> {
    let setup = if required {
        Some(vlm::ensure(
            cli.llama_server.as_deref(),
            cli.vlm_dir.clone(),
            cli.yes,
        )?)
    } else {
        vlm::ready(cli.llama_server.as_deref(), cli.vlm_dir.clone())
    };
    let Some(setup) = setup else {
        return Ok(None);
    };
    let server = vlm::Server::start(&setup.server, &setup.files)?;
    server.read(img).map(Some)
}

/// The small formula model, with tidy and prose mode.
fn read_small(cli: &Cli, img: &DynamicImage) -> Result<Outcome> {
    let (dir, mut spec) = fetch::locate(cli.model_dir.clone())?;
    if let Some(fit) = cli.fit {
        spec.fit = fit;
    }
    let mut rec = model::Recognizer::load(&dir, spec)?;
    let result = rec.recognize(img, cli.max_tokens)?;
    let (latex, kind) = if cli.raw {
        (result.text.clone(), tidy::Kind::Math)
    } else {
        tidy::tidy_with_kind(&result.text)
    };
    Ok(Outcome {
        latex,
        kind,
        min_prob: Some(result.min_prob()),
        mean_logprob: Some(result.mean_logprob()),
        raw: result.text,
        engine: "small",
        stop: result.stop,
        tokens: result.tokens,
    })
}

#[derive(Serialize)]
struct JsonOut<'a> {
    latex: &'a str,
    kind: tidy::Kind,
    raw: &'a str,
    /// "small" or "vlm"
    engine: &'a str,
    stop: model::Stop,
    /// Only the small model reports per-token confidence.
    min_prob: Option<f32>,
    mean_logprob: Option<f32>,
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
