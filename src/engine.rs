//! Choosing and running the engines: the one place both the command line and the window call.
//!
//! `Options` says what to do; `Warm` holds the expensive things (the running `llama-server`, the
//! loaded small model) so a long-lived caller like the window can reuse them between reads. The
//! command line makes a fresh `Warm` per run and drops it, so nothing outlives the command.

use std::path::PathBuf;

use anyhow::Result;
use clap::ValueEnum;
use image::DynamicImage;

use crate::{fetch, model, spec::Fit, tidy, ui, vlm};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Engine {
    /// The vision engine when it is set up, otherwise the small model
    Auto,
    /// Only the small formula model (fast, 118 MB)
    Small,
    /// Only the vision engine (OvisOCR2 via llama.cpp, about 1 GB)
    Vlm,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Engine::Auto => "auto",
            Engine::Small => "small",
            Engine::Vlm => "vlm",
        }
    }
}

/// Everything that decides how an image is read.
pub struct Options {
    pub engine: Engine,
    /// Skip cleanup and keep the model's output as-is
    pub raw: bool,
    pub fit: Option<Fit>,
    pub model_dir: Option<PathBuf>,
    pub max_tokens: usize,
    pub llama_server: Option<PathBuf>,
    pub vlm_dir: Option<PathBuf>,
    /// Answer yes when the vision engine must be set up first (command line only)
    pub yes: bool,
    /// Don't print tips and warnings (the window shows its own)
    pub quiet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            engine: Engine::Auto,
            raw: false,
            fit: None,
            model_dir: None,
            max_tokens: 512,
            llama_server: None,
            vlm_dir: None,
            yes: false,
            quiet: true,
        }
    }
}

/// What an engine read, in one shape.
pub struct Outcome {
    pub latex: String,
    pub kind: tidy::Kind,
    pub raw: String,
    pub engine: &'static str,
    pub stop: model::Stop,
    pub min_prob: Option<f32>,
    pub mean_logprob: Option<f32>,
    pub tokens: Vec<model::Token>,
}

/// Loaded engines kept between reads.
#[derive(Default)]
pub struct Warm {
    server: Option<vlm::Server>,
    recognizer: Option<model::Recognizer>,
}

impl Warm {
    /// Stop the helper and unload the models, freeing their memory.
    pub fn cool_down(&mut self) {
        self.server = None;
        self.recognizer = None;
    }

    pub fn is_warm(&self) -> bool {
        self.server.is_some() || self.recognizer.is_some()
    }
}

/// Choose and run the engines. The vision engine goes first: if it finds text, that is the answer.
/// If it finds only a formula, the small model is preferred (it is better on isolated formulas),
/// with the vision engine's formula as the fallback.
pub fn read(
    opts: &Options,
    img: &DynamicImage,
    warm: &mut Warm,
    report: ui::Reporter,
) -> Result<Outcome> {
    let vision = match opts.engine {
        Engine::Small => None,
        Engine::Vlm => run_vlm(opts, img, true, warm)?,
        Engine::Auto => match run_vlm(opts, img, false, warm) {
            Ok(None) => {
                if !opts.quiet {
                    eprintln!(
                        "tip: run `texcopy --setup` once to enable the vision engine, which reads whole paragraphs and long equations far better (a one-time download, it asks first)"
                    );
                }
                None
            }
            Ok(some) => some,
            Err(e) => {
                if !opts.quiet {
                    eprintln!("warning: the vision engine failed ({e:#}); using the small model");
                }
                None
            }
        },
    };
    let Some((raw, truncated)) = vision else {
        return read_small(opts, img, warm, report);
    };
    let stop = if truncated {
        model::Stop::MaxTokens
    } else {
        model::Stop::Eos
    };
    let from_vision = |latex: String, kind| Outcome {
        latex: if opts.raw { raw.clone() } else { latex },
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
        vlm::Reading::Formula(f) if opts.engine == Engine::Vlm => {
            Ok(from_vision(f, tidy::Kind::Math))
        }
        vlm::Reading::Formula(f) => {
            let small = read_small(opts, img, warm, report)?;
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
/// false; when it is required, a missing piece is set up first (asking the user in the terminal).
fn run_vlm(
    opts: &Options,
    img: &DynamicImage,
    required: bool,
    warm: &mut Warm,
) -> Result<Option<(String, bool)>> {
    if warm.server.is_none() {
        let setup = if required {
            Some(vlm::ensure(
                opts.llama_server.as_deref(),
                opts.vlm_dir.clone(),
                opts.yes,
            )?)
        } else {
            vlm::ready(opts.llama_server.as_deref(), opts.vlm_dir.clone())
        };
        let Some(setup) = setup else {
            return Ok(None);
        };
        warm.server = Some(vlm::Server::start(&setup.server, &setup.files)?);
    }
    let server = warm.server.as_ref().expect("started above");
    match server.read(img) {
        Ok(read) => Ok(Some(read)),
        Err(e) => {
            // A helper that died or hung must not be reused for the next read.
            warm.server = None;
            Err(e)
        }
    }
}

/// The small formula model, with tidy and prose mode.
fn read_small(
    opts: &Options,
    img: &DynamicImage,
    warm: &mut Warm,
    report: ui::Reporter,
) -> Result<Outcome> {
    if warm.recognizer.is_none() {
        let (dir, mut spec) = fetch::locate(opts.model_dir.clone(), report)?;
        if let Some(fit) = opts.fit {
            spec.fit = fit;
        }
        warm.recognizer = Some(model::Recognizer::load(&dir, spec)?);
    }
    let rec = warm.recognizer.as_mut().expect("loaded above");
    let result = rec.recognize(img, opts.max_tokens)?;
    let (latex, kind) = if opts.raw {
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
