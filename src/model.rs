//! Image -> LaTeX recognition with an ONNX vision encoder-decoder (see `spec.rs`).
//!
//! The model is a vision encoder-decoder: a ViT turns the image into a grid of
//! feature vectors, and a small transformer decoder writes LaTeX tokens one at a
//! time while attending to those vectors.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use image::{DynamicImage, RgbImage, imageops::FilterType};
use ort::{
    session::Session,
    value::{Tensor, TensorRef},
};
use serde::Serialize;
use tokenizers::Tokenizer;

use crate::spec::{Fit, ModelSpec};

/// Why decoding stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    /// The model finished (emitted its end-of-sequence token).
    Eos,
    /// Hit the token limit before finishing: the output is cut off.
    MaxTokens,
    /// The model got stuck repeating the same few tokens; the output is garbage.
    Looped,
}

/// One generated token and the model's probability for having chosen it.
#[derive(Debug, Clone, Serialize)]
pub struct Token {
    pub text: String,
    pub prob: f32,
}

#[derive(Debug, Clone)]
pub struct Recognition {
    /// The raw detokenized output (see `tidy` for turning it into idiomatic LaTeX).
    pub text: String,
    pub tokens: Vec<Token>,
    pub stop: Stop,
}

impl Recognition {
    /// Probability of the least certain token (1.0 when there are none).
    pub fn min_prob(&self) -> f32 {
        self.tokens.iter().map(|t| t.prob).fold(1.0, f32::min)
    }

    /// Average log-probability per token (0.0 is perfectly certain, more negative is worse).
    pub fn mean_logprob(&self) -> f32 {
        if self.tokens.is_empty() {
            return 0.0;
        }
        self.tokens
            .iter()
            .map(|t| t.prob.max(1e-9).ln())
            .sum::<f32>()
            / self.tokens.len() as f32
    }
}

/// Tokens the model was less sure of than this are worth proofreading. Chosen from the
/// benchmark (tools/bench.py): at 0.5 none of the 44 exactly-recovered formulas were flagged;
/// at 0.7 a quarter of them were.
pub const LOW_CONFIDENCE: f32 = 0.5;

/// Index and softmax probability of the highest logit.
fn argmax_prob(logits: &[f32]) -> (i64, f32) {
    let (i, &max) = logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap();
    let z: f32 = logits.iter().map(|l| (l - max).exp()).sum();
    (i as i64, 1.0 / z)
}

/// True if the tail of `ids` is a short chunk (period 1..=12) repeated back-to-back at
/// least 4 times and across at least 32 tokens. Real LaTeX can repeat (`0&0&0&0`), but not
/// this much without the digits or indices changing.
fn looped(ids: &[i64]) -> bool {
    (1..=12usize).any(|p| {
        let repeats = 4.max(32usize.div_ceil(p));
        let span = p * repeats;
        ids.len() >= span && ids[ids.len() - span..].windows(p + 1).all(|w| w[0] == w[p])
    })
}

pub struct Recognizer {
    spec: ModelSpec,
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
}

impl Recognizer {
    pub fn load(dir: &Path, spec: ModelSpec) -> Result<Self> {
        let session = |name: &str| -> Result<Session> {
            let path = dir.join(name);
            Session::builder()?
                .commit_from_file(&path)
                .with_context(|| format!("loading {}", path.display()))
        };
        let tok_path = dir.join(&spec.tokenizer);
        Ok(Self {
            encoder: session(&spec.encoder)?,
            decoder: session(&spec.decoder)?,
            tokenizer: Tokenizer::from_file(&tok_path)
                .map_err(|e| anyhow!("loading {}: {e}", tok_path.display()))?,
            spec,
        })
    }

    /// Recognise a single formula image (no `$` delimiters).
    pub fn recognize(&mut self, img: &DynamicImage, max_tokens: usize) -> Result<Recognition> {
        // 1. Encode the image once.
        let (w, h) = (
            self.spec.input_width as usize,
            self.spec.input_height as usize,
        );
        let pixels = preprocess(img, &self.spec);
        let enc_out = self.encoder.run(ort::inputs![
            "pixel_values" => Tensor::from_array(([1usize, 3, h, w], pixels))?
        ])?;
        let (enc_shape, enc_data) = enc_out["last_hidden_state"].try_extract_tensor::<f32>()?;
        let enc_shape: Vec<usize> = enc_shape.iter().map(|&d| d as usize).collect();
        let enc_data = enc_data.to_vec();
        drop(enc_out);

        // 2. Greedy autoregressive decoding. The exported decoder has no KV cache,
        //    so every step feeds the full prefix again.
        let mut ids = vec![self.spec.start_token];
        let mut probs: Vec<f32> = Vec::new();
        let mut stop = Stop::MaxTokens;
        for _ in 0..max_tokens {
            let out = self.decoder.run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, ids.len()], ids.clone()))?,
                "encoder_hidden_states" => TensorRef::from_array_view((enc_shape.as_slice(), enc_data.as_slice()))?
            ])?;
            let (shape, logits) = out["logits"].try_extract_tensor::<f32>()?;
            let vocab = shape[2] as usize;
            let (next, prob) = argmax_prob(&logits[(ids.len() - 1) * vocab..ids.len() * vocab]);
            if next == self.spec.eos_token {
                stop = Stop::Eos;
                break;
            }
            ids.push(next);
            probs.push(prob);
            if looped(&ids[1..]) {
                stop = Stop::Looped;
                break;
            }
        }

        // 3. Token ids -> text (drop the start token).
        let ids: Vec<u32> = ids[1..].iter().map(|&i| i as u32).collect();
        let decode = |ids: &[u32]| {
            self.tokenizer
                .decode(ids, true)
                .map_err(|e| anyhow!("decoding tokens: {e}"))
        };
        let tokens = ids
            .iter()
            .zip(&probs)
            .map(|(&id, &prob)| {
                Ok(Token {
                    text: decode(&[id])?,
                    prob,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Recognition {
            text: decode(&ids)?,
            tokens,
            stop,
        })
    }
}

/// Turn an arbitrary screenshot into the CHW float tensor the encoder wants:
/// flatten alpha onto white, flip dark-mode captures to dark-on-light, fit it to
/// the model's input size (stretch, or pad to its aspect ratio first), then
/// normalise with the spec's per-channel mean and std.
fn preprocess(img: &DynamicImage, spec: &ModelSpec) -> Vec<f32> {
    let (tw, th) = (spec.input_width, spec.input_height);
    let rgba = img.to_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as f32 / 255.0;
        let blend = |c: u8| (c as f32 * a + 255.0 * (1.0 - a)).round() as u8;
        rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }

    // The model was trained on dark ink on a light page.
    let luma_sum: f64 = rgb
        .pixels()
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .sum();
    if luma_sum / (rgb.width() as f64 * rgb.height() as f64) < 127.5 {
        image::imageops::invert(&mut rgb);
    }

    let resized = match spec.fit {
        Fit::Stretch => image::imageops::resize(&rgb, tw, th, FilterType::CatmullRom),
        Fit::Pad => {
            // Pad with the page colour (taken from the top-left corner) up to the target aspect ratio.
            let (w, h) = (rgb.width(), rgb.height());
            let (cw, ch) = if w as u64 * th as u64 >= h as u64 * tw as u64 {
                (w, (w as u64 * th as u64).div_ceil(tw as u64) as u32)
            } else {
                ((h as u64 * tw as u64).div_ceil(th as u64) as u32, h)
            };
            let mut canvas = RgbImage::from_pixel(cw, ch, *rgb.get_pixel(0, 0));
            image::imageops::overlay(
                &mut canvas,
                &rgb,
                ((cw - w) / 2) as i64,
                ((ch - h) / 2) as i64,
            );
            image::imageops::resize(&canvas, tw, th, FilterType::CatmullRom)
        }
    };

    let plane = (tw * th) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, p) in resized.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (p[c] as f32 / 255.0 - spec.mean[c]) / spec.std[c];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_prob_is_a_softmax() {
        let (i, p) = argmax_prob(&[0.0, 5.0, 0.0]);
        assert_eq!(i, 1);
        assert!((p - 5f32.exp() / (5f32.exp() + 2.0)).abs() < 1e-6);
        assert!((argmax_prob(&[3.0])).1 > 0.999);
    }

    #[test]
    fn detects_runaway_loops_but_not_legitimate_repetition() {
        let mut stuck: Vec<i64> = (1..=10).collect();
        stuck.extend([7, 8, 9].repeat(12)); // a 3-token chunk repeated 12 times
        assert!(looped(&stuck));
        assert!(looped(&[5; 40])); // one token repeated forever

        // `0 & 0 & ... ` for a 6-column zero row: period 2, only 6 repeats.
        assert!(!looped(&[0, 1].repeat(6)));
        // Repetition with a changing index (a_1 x_1 + a_2 x_2 ...) never matches exactly.
        let terms: Vec<i64> = (1..=20).flat_map(|k| [10, 11, 100 + k, 12]).collect();
        assert!(!looped(&terms));
        assert!(!looped(&[]));
    }

    #[test]
    fn confidence_summaries() {
        let tok = |p| Token {
            text: "x".into(),
            prob: p,
        };
        let r = Recognition {
            text: String::new(),
            tokens: vec![tok(1.0), tok(0.5)],
            stop: Stop::Eos,
        };
        assert_eq!(r.min_prob(), 0.5);
        assert!((r.mean_logprob() - 0.5f32.ln() / 2.0).abs() < 1e-6);
        let empty = Recognition {
            text: String::new(),
            tokens: vec![],
            stop: Stop::Eos,
        };
        assert_eq!((empty.min_prob(), empty.mean_logprob()), (1.0, 0.0));
    }
}
