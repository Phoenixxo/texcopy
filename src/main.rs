mod capture;
mod clipboard;
mod engine;
mod fetch;
#[cfg(feature = "gui")]
mod gui;
mod helper;
mod model;
mod prose;
mod spec;
mod tidy;
mod ui;
mod vlm;

use std::{io::Read, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use engine::{Engine, Wrap};
use image::DynamicImage;
use serde::Serialize;

/// Select something LaTeX-rendered on screen and get its LaTeX source on your clipboard.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Image file to read. With no input option, the window opens.
    image: Option<PathBuf>,

    /// Drag-select a screen region in the terminal instead of opening the window (macOS only for now)
    #[arg(long, conflicts_with_all = ["image", "clipboard", "screen"])]
    select: bool,

    /// Internal: open the window, "capture" this picture, check the result, exit
    #[arg(long, hide = true, value_name = "PNG")]
    gui_selftest: Option<PathBuf>,

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

    #[cfg(feature = "gui")]
    if cli.image.is_none() && !cli.clipboard && !cli.screen && !cli.select {
        let opts = engine::Options {
            engine: cli.engine,
            raw: cli.raw,
            fit: cli.fit,
            model_dir: cli.model_dir.clone(),
            max_tokens: cli.max_tokens,
            llama_server: cli.llama_server.clone(),
            vlm_dir: cli.vlm_dir.clone(),
            yes: false,
            quiet: true,
        };
        return gui::run(opts, cli.gui_selftest.clone());
    }

    // Grab the image first so a cancelled selection costs nothing.
    let Some(img) = load_image(&cli)? else {
        eprintln!("Cancelled.");
        return Ok(());
    };

    let opts = engine::Options {
        engine: cli.engine,
        raw: cli.raw,
        fit: cli.fit,
        model_dir: cli.model_dir.clone(),
        max_tokens: cli.max_tokens,
        llama_server: cli.llama_server.clone(),
        vlm_dir: cli.vlm_dir.clone(),
        yes: cli.yes,
        quiet: cli.json,
    };
    let mut term = ui::Terminal::default();
    let outcome = engine::read(&opts, &img, &mut engine::Warm::default(), &mut |e| {
        term.event(e)
    })?;

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

    let out = engine::wrapped(cli.wrap, kind, &latex);
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
