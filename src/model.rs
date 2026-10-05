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
use tokenizers::Tokenizer;

use crate::spec::ModelSpec;

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

    /// Recognise a single formula image. Returns the model's raw token stream joined by
    /// spaces (no `$` delimiters); see `tidy` for turning it into idiomatic LaTeX.
    pub fn recognize(&mut self, img: &DynamicImage, max_tokens: usize) -> Result<String> {
        // 1. Encode the image once.
        let (w, h) = (self.spec.input_width as usize, self.spec.input_height as usize);
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
        for _ in 0..max_tokens {
            let out = self.decoder.run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, ids.len()], ids.clone()))?,
                "encoder_hidden_states" => TensorRef::from_array_view((enc_shape.as_slice(), enc_data.as_slice()))?
            ])?;
            let (shape, logits) = out["logits"].try_extract_tensor::<f32>()?;
            let vocab = shape[2] as usize;
            let last = &logits[(ids.len() - 1) * vocab..ids.len() * vocab];
            let next = last
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i as i64)
                .unwrap();
            if next == self.spec.eos_token {
                break;
            }
            ids.push(next);
        }

        // 3. Token ids -> text (drop the start token).
        let ids: Vec<u32> = ids[1..].iter().map(|&i| i as u32).collect();
        let text = self
            .tokenizer
            .decode(&ids, true)
            .map_err(|e| anyhow!("decoding tokens: {e}"))?;
        Ok(text)
    }
}

/// Turn an arbitrary screenshot into the CHW float tensor the encoder wants:
/// flatten alpha onto white, flip dark-mode captures to dark-on-light, pad to the
/// model's aspect ratio (so formulas keep their proportions), resize, then
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

    // Pad with the page colour (taken from the top-left corner) up to the target aspect ratio.
    let (w, h) = (rgb.width(), rgb.height());
    let (cw, ch) = if w as u64 * th as u64 >= h as u64 * tw as u64 {
        (w, (w as u64 * th as u64).div_ceil(tw as u64) as u32)
    } else {
        ((h as u64 * tw as u64).div_ceil(th as u64) as u32, h)
    };
    let mut canvas = RgbImage::from_pixel(cw, ch, *rgb.get_pixel(0, 0));
    image::imageops::overlay(&mut canvas, &rgb, ((cw - w) / 2) as i64, ((ch - h) / 2) as i64);
    let resized = image::imageops::resize(&canvas, tw, th, FilterType::CatmullRom);

    let plane = (tw * th) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, p) in resized.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (p[c] as f32 / 255.0 - spec.mean[c]) / spec.std[c];
        }
    }
    out
}
