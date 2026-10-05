//! Image -> LaTeX recognition with the pix2text-mfr ONNX model.
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

/// The encoder expects a square image of this side length.
const INPUT_SIZE: u32 = 384;
/// `decoder_start_token_id` from generation_config.json (same id as EOS).
const START_TOKEN: i64 = 2;
const EOS_TOKEN: i64 = 2;

pub struct Recognizer {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
}

impl Recognizer {
    pub fn load(dir: &Path) -> Result<Self> {
        let session = |name: &str| -> Result<Session> {
            let path = dir.join(name);
            Session::builder()?
                .commit_from_file(&path)
                .with_context(|| format!("loading {}", path.display()))
        };
        let tok_path = dir.join("tokenizer.json");
        Ok(Self {
            encoder: session("encoder_model.onnx")?,
            decoder: session("decoder_model.onnx")?,
            tokenizer: Tokenizer::from_file(&tok_path)
                .map_err(|e| anyhow!("loading {}: {e}", tok_path.display()))?,
        })
    }

    /// Recognise a single formula image. Returns the model's raw token stream joined by
    /// spaces (no `$` delimiters); see `tidy` for turning it into idiomatic LaTeX.
    pub fn recognize(&mut self, img: &DynamicImage, max_tokens: usize) -> Result<String> {
        // 1. Encode the image once.
        let pixels = preprocess(img);
        let n = INPUT_SIZE as usize;
        let enc_out = self.encoder.run(ort::inputs![
            "pixel_values" => Tensor::from_array(([1usize, 3, n, n], pixels))?
        ])?;
        let (enc_shape, enc_data) = enc_out["last_hidden_state"].try_extract_tensor::<f32>()?;
        let enc_shape: Vec<usize> = enc_shape.iter().map(|&d| d as usize).collect();
        let enc_data = enc_data.to_vec();
        drop(enc_out);

        // 2. Greedy autoregressive decoding. The exported decoder has no KV cache,
        //    so every step feeds the full prefix again.
        let mut ids = vec![START_TOKEN];
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
            if next == EOS_TOKEN {
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
/// flatten alpha onto white, flip dark-mode captures to dark-on-light, pad to a
/// square (so wide formulas keep their aspect ratio), resize, then normalise to
/// [-1, 1] like the model's DeiT image processor (mean = std = 0.5).
fn preprocess(img: &DynamicImage) -> Vec<f32> {
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

    // Pad to a square with the page colour (taken from the top-left corner).
    let side = rgb.width().max(rgb.height());
    let bg = *rgb.get_pixel(0, 0);
    let mut square = RgbImage::from_pixel(side, side, bg);
    image::imageops::overlay(
        &mut square,
        &rgb,
        ((side - rgb.width()) / 2) as i64,
        ((side - rgb.height()) / 2) as i64,
    );
    let resized = image::imageops::resize(&square, INPUT_SIZE, INPUT_SIZE, FilterType::CatmullRom);

    let plane = (INPUT_SIZE * INPUT_SIZE) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, p) in resized.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (p[c] as f32 / 255.0 - 0.5) / 0.5;
        }
    }
    out
}
