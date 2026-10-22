mod capture;
mod clipboard;
mod convert;
mod engine;
mod fetch;
#[cfg(feature = "gui")]
mod gui;
mod helper;
mod model;
#[cfg(all(feature = "gui", feature = "capture"))]
mod overlay;
mod prose;
mod spec;
mod table;
mod tidy;
mod ui;
mod uninstall;
mod vlm;

use std::{
    io::{Read, Write},
    path::PathBuf,
};

use anyhow::{Context, Result};
use clap::Parser;
use engine::{Engine, Mode, Wrap};
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

    /// Read every image listed in this file (one path per line) and print one JSON line each, keeping
    /// the engines loaded between images. Used by the benchmark, where starting the vision model for
    /// every image would take most of the time.
    #[arg(long, hide = true, value_name = "LIST")]
    batch: Option<PathBuf>,

    /// Internal: run the built-in region selector and write the cropped PNG here (see overlay.rs)
    #[arg(long, hide = true, value_name = "PNG")]
    select_overlay: Option<PathBuf>,

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

    /// Remove the models, the llama.cpp helper and the saved settings texcopy downloaded, then exit
    #[arg(long, conflicts_with_all = ["image", "clipboard", "screen", "select", "setup", "batch"])]
    uninstall: bool,

    /// With --uninstall, list what would be removed and remove nothing
    #[arg(long, requires = "uninstall")]
    dry_run: bool,

    /// Directory with the vision model files (model.gguf and mmproj.gguf)
    #[arg(long, env = "TEXCOPY_VLM_DIR")]
    vlm_dir: Option<PathBuf>,

    /// Path to llama-server from llama.cpp (default: the one on PATH)
    #[arg(long, env = "TEXCOPY_LLAMA_SERVER")]
    llama_server: Option<PathBuf>,

    /// Directory containing the model files and optional spec.json (default: ./models, else the cache dir)
    #[arg(long, env = "TEXCOPY_MODEL_DIR")]
    model_dir: Option<PathBuf>,

    /// How the output reads. human: cleaned up to look hand-written (the default). exact: the
    /// model's text character for character
    #[arg(long, value_enum, env = "TEXCOPY_MODE")]
    mode: Option<Mode>,

    /// Same as --mode exact
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

impl Cli {
    /// --raw wins, then --mode, then human.
    fn mode(&self) -> Mode {
        if self.raw {
            Mode::Exact
        } else {
            self.mode.unwrap_or_default()
        }
    }
}

/// the whole program, texcopy and texcopy-gui are thin launchers around this.
pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let mode = cli.mode();
    if cli.hold_clipboard {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        return clipboard::hold(&text);
    }

    #[cfg(all(feature = "gui", feature = "capture"))]
    if let Some(out) = cli.select_overlay.clone() {
        return overlay::run(out);
    }

    if cli.uninstall {
        return uninstall::run(cli.dry_run, cli.yes);
    }

    if cli.setup {
        vlm::ensure(cli.llama_server.as_deref(), cli.vlm_dir.clone(), cli.yes)?;
        eprintln!("Try it:  texcopy --engine vlm some-image.png");
        return Ok(());
    }

    if let Some(list) = cli.batch.clone() {
        return run_batch(&cli, &list, mode);
    }

    #[cfg(feature = "gui")]
    if cli.image.is_none() && !cli.clipboard && !cli.screen && !cli.select {
        let opts = engine::Options {
            engine: cli.engine,
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

    // grab the image first so a cancelled selection costs nothing.
    let Some(img) = load_image(&cli)? else {
        eprintln!("Cancelled.");
        return Ok(());
    };

    let opts = engine::Options {
        engine: cli.engine,
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
        let (shown, shown_kind) = outcome.view(mode);
        let out = JsonOut {
            latex: shown,
            kind: shown_kind,
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
            // show a little context before the marked token.
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

    let (latex, kind) = outcome.view(mode);
    if kind == tidy::Kind::Prose && prose::has_run_together_words(latex) {
        eprintln!(
            "warning: some words look run together (the model lost the spaces). Try selecting one line at a time."
        );
    }
    if outcome.engine == "small" && latex.contains("\\nu") {
        eprintln!(
            "note: the model read a \\nu. In an italic font, v and \u{3bd} look nearly identical, so check which one it should be."
        );
    }

    let out = engine::wrapped(cli.wrap, kind, latex);
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
    /// small or vlm.
    engine: &'a str,
    stop: model::Stop,
    /// only the small model reports per token confidence.
    min_prob: Option<f32>,
    mean_logprob: Option<f32>,
    tokens: &'a [model::Token],
}

/// one line of --batch output, the path then the --json fields.
#[derive(Serialize)]
struct BatchOut<'a> {
    path: &'a str,
    #[serde(flatten)]
    out: JsonOut<'a>,
}

/// --batch reads each listed image with one set of engines and prints a JSON line each, errors print an error line and the run goes on.
fn run_batch(cli: &Cli, list: &std::path::Path, mode: Mode) -> Result<()> {
    let opts = engine::Options {
        engine: cli.engine,
        fit: cli.fit,
        model_dir: cli.model_dir.clone(),
        max_tokens: cli.max_tokens,
        llama_server: cli.llama_server.clone(),
        vlm_dir: cli.vlm_dir.clone(),
        yes: cli.yes,
        quiet: true,
    };
    let text =
        std::fs::read_to_string(list).with_context(|| format!("reading {}", list.display()))?;
    let mut warm = engine::Warm::default();
    for path in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let result = image::open(path)
            .with_context(|| format!("opening {path}"))
            .and_then(|img| engine::read(&opts, &img, &mut warm, &mut ui::silent));
        match result {
            Ok(o) => {
                let (shown, shown_kind) = o.view(mode);
                let line = BatchOut {
                    path,
                    out: JsonOut {
                        latex: shown,
                        kind: shown_kind,
                        raw: &o.raw,
                        engine: o.engine,
                        stop: o.stop,
                        min_prob: o.min_prob,
                        mean_logprob: o.mean_logprob,
                        tokens: &o.tokens,
                    },
                };
                println!("{}", serde_json::to_string(&line)?);
            }
            Err(e) => println!(
                "{}",
                serde_json::json!({"path": path, "error": format!("{e:#}")})
            ),
        }
        std::io::stdout().flush().ok();
    }
    Ok(())
}

/// None means the user cancelled.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_of(args: &[&str]) -> Mode {
        let mut argv = vec!["texcopy"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).unwrap().mode()
    }

    #[test]
    fn the_mode_is_human_unless_asked_otherwise() {
        assert!(mode_of(&[]) == Mode::Human);
        assert!(mode_of(&["--mode", "human"]) == Mode::Human);
        assert!(mode_of(&["--mode", "exact"]) == Mode::Exact);
    }

    #[test]
    fn raw_is_a_shortcut_for_exact_and_wins_over_mode() {
        assert!(mode_of(&["--raw"]) == Mode::Exact);
        assert!(mode_of(&["--raw", "--mode", "human"]) == Mode::Exact);
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        assert!(Cli::try_parse_from(["texcopy", "--mode", "loose"]).is_err());
    }

    #[test]
    fn a_batch_line_has_the_path_and_the_same_fields_as_json_output() {
        let out = JsonOut {
            latex: "x^2",
            kind: tidy::Kind::Math,
            raw: "x ^ 2",
            engine: "small",
            stop: model::Stop::Eos,
            min_prob: Some(0.9),
            mean_logprob: None,
            tokens: &[],
        };
        let line = serde_json::to_value(BatchOut { path: "a.png", out }).unwrap();
        assert_eq!(line["path"], "a.png");
        assert_eq!(line["latex"], "x^2");
        assert_eq!(line["engine"], "small");
        assert!(
            line.get("out").is_none(),
            "the fields sit beside the path, not nested under it"
        );
    }
}
