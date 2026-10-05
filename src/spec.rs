//! Everything model-specific lives here, so a different encoder-decoder (or one you
//! train yourself) is a folder of ONNX files plus an optional `spec.json`, not a code change.
//!
//! The ONNX graphs must follow the Hugging Face `optimum` export convention for
//! VisionEncoderDecoder models: the encoder maps `pixel_values` to
//! `last_hidden_state`, and the decoder maps `input_ids` + `encoder_hidden_states`
//! to `logits` (no KV cache).

use std::{fs, path::Path};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Deserialize;

/// How an image of any shape is turned into the encoder's fixed input size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Squash to the input size, ignoring aspect ratio (what the model's own image processor does)
    #[default]
    Stretch,
    /// Pad with the page colour to the input aspect ratio, then shrink
    Pad,
}

/// Any field omitted from `spec.json` keeps its pix2text-mfr default.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModelSpec {
    pub name: String,
    pub encoder: String,
    pub decoder: String,
    pub tokenizer: String,
    /// Size the encoder expects, in pixels.
    pub input_width: u32,
    pub input_height: u32,
    /// Per-channel normalisation: `(pixel / 255 - mean) / std`.
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub fit: Fit,
    pub start_token: i64,
    pub eos_token: i64,
    /// Where to download missing files from (`{base_url}/{file}`); `None` for local-only models.
    pub base_url: Option<String>,
}

impl Default for ModelSpec {
    fn default() -> Self {
        Self {
            name: "pix2text-mfr".into(),
            encoder: "encoder_model.onnx".into(),
            decoder: "decoder_model.onnx".into(),
            tokenizer: "tokenizer.json".into(),
            input_width: 384,
            input_height: 384,
            mean: [0.5; 3],
            std: [0.5; 3],
            fit: Fit::Stretch,
            start_token: 2,
            eos_token: 2,
            base_url: Some("https://huggingface.co/breezedeus/pix2text-mfr/resolve/main".into()),
        }
    }
}

impl ModelSpec {
    /// `dir/spec.json` if it exists, otherwise the built-in default.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("spec.json");
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn files(&self) -> [&str; 3] {
        [&self.encoder, &self.decoder, &self.tokenizer]
    }

    pub fn is_complete(&self, dir: &Path) -> bool {
        self.files().iter().all(|f| dir.join(f).is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::ModelSpec;

    #[test]
    fn partial_json_keeps_defaults() {
        let s: ModelSpec =
            serde_json::from_str(r#"{"name":"mine","input_width":672,"input_height":192}"#)
                .unwrap();
        assert_eq!(
            (s.input_width, s.input_height, s.name.as_str()),
            (672, 192, "mine")
        );
        assert_eq!(s.encoder, "encoder_model.onnx");
        assert_eq!(s.start_token, 2);
    }
}
