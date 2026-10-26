//! picks and runs the engines, the command line and the window both call this.

use std::path::PathBuf;

use anyhow::Result;
use clap::ValueEnum;
use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::{fetch, model, spec::Fit, tidy, ui, vlm};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// vision engine if it's set up, otherwise the small model.
    Auto,
    /// only the small formula model.
    Small,
    /// only the vision engine.
    Vlm,
}

/// what to put around a formula.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wrap {
    /// raw LaTeX.
    None,
    /// $...$
    Inline,
    /// \[ ... \]
    Display,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// cleaned up to read like LaTeX a person wrote.
    #[default]
    Human,
    /// exactly what the model wrote.
    Exact,
}

/// sentences already have their own $...$ so this only applies to formulas.
pub fn wrapped(wrap: Wrap, kind: tidy::Kind, latex: &str) -> String {
    match (wrap, kind) {
        (Wrap::None, _) | (_, tidy::Kind::Prose) => latex.to_string(),
        (Wrap::Inline, _) => format!("${latex}$"),
        (Wrap::Display, _) => format!("\\[ {latex} \\]"),
    }
}

/// everything that decides how an image is read.
#[derive(Clone)]
pub struct Options {
    pub engine: Engine,
    pub fit: Option<Fit>,
    pub model_dir: Option<PathBuf>,
    pub max_tokens: usize,
    pub llama_server: Option<PathBuf>,
    pub vlm_dir: Option<PathBuf>,
    /// say yes when the vision engine needs setting up, command line only.
    pub yes: bool,
    /// no tips or warnings, the window shows its own.
    pub quiet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            engine: Engine::Auto,
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

/// what an engine read.
pub struct Outcome {
    /// the cleaned text, what human mode shows.
    pub latex: String,
    pub kind: tidy::Kind,
    /// the model's own text, what exact mode shows.
    pub raw: String,
    pub raw_kind: tidy::Kind,
    pub engine: &'static str,
    pub stop: model::Stop,
    pub min_prob: Option<f32>,
    pub mean_logprob: Option<f32>,
    pub tokens: Vec<model::Token>,
}

impl Outcome {
    pub fn view(&self, mode: Mode) -> (&str, tidy::Kind) {
        match mode {
            Mode::Human => (&self.latex, self.kind),
            Mode::Exact => (&self.raw, self.raw_kind),
        }
    }
}

/// engines kept loaded between reads.
#[derive(Default)]
pub struct Warm {
    server: Option<vlm::Server>,
    recognizer: Option<model::Recognizer>,
}

#[cfg_attr(not(feature = "gui"), allow(dead_code))] // only the window keeps engines warm.
impl Warm {
    /// stop the helper and unload the models.
    pub fn cool_down(&mut self) {
        self.server = None;
        self.recognizer = None;
    }

    /// start the vision engine now if it's set up, never downloads or asks.
    pub fn prewarm(&mut self, opts: &Options) -> Result<bool> {
        if opts.engine == Engine::Small || self.server.is_some() {
            return Ok(self.server.is_some());
        }
        let Some(setup) = vlm::ready(opts.llama_server.as_deref(), opts.vlm_dir.clone()) else {
            return Ok(false);
        };
        self.server = Some(vlm::Server::start(&setup.server, &setup.files)?);
        Ok(true)
    }

    pub fn is_warm(&self) -> bool {
        self.server.is_some() || self.recognizer.is_some()
    }
}

/// pick and run the engines, vision first and the small model preferred for plain formulas.
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
        latex,
        kind,
        raw: raw.clone(),
        raw_kind: kind,
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
            // the small model giving up is when the vision formula is better.
            Ok(if small.stop == model::Stop::Looped {
                from_vision(f, tidy::Kind::Math)
            } else {
                small
            })
        }
    }
}

/// run the vision engine, None if it isn't set up and that's allowed.
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
            // a helper that died or hung doesn't get reused.
            warm.server = None;
            Err(e)
        }
    }
}

/// the small formula model with tidy and prose mode.
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
    let (latex, kind) = tidy::tidy_with_kind(&result.text);
    Ok(Outcome {
        latex,
        kind,
        raw_kind: tidy::Kind::Math,
        min_prob: Some(result.min_prob()),
        mean_logprob: Some(result.mean_logprob()),
        raw: result.text,
        engine: "small",
        stop: result.stop,
        tokens: result.tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_applies_to_formulas_but_never_to_sentences() {
        assert_eq!(wrapped(Wrap::None, tidy::Kind::Math, "x^2"), "x^2");
        assert_eq!(wrapped(Wrap::Inline, tidy::Kind::Math, "x^2"), "$x^2$");
        assert_eq!(
            wrapped(Wrap::Display, tidy::Kind::Math, "x^2"),
            "\\[ x^2 \\]"
        );
        let sentence = "Prove that $v$ is a cut-vertex.";
        assert_eq!(wrapped(Wrap::Inline, tidy::Kind::Prose, sentence), sentence);
    }
}
