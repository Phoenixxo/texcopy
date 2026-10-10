//! What gets written next to the pictures: one entry per case. The benchmark reads this file.

use serde::{Deserialize, Serialize};

/// How a case is graded: a formula is shown as display math, a document is a block of text.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Grading {
    Formula,
    Document,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Case {
    pub name: String,
    pub category: String,
    pub kind: Grading,
    /// The exact LaTeX the picture was made from.
    pub truth: String,
    pub renderer: String,
    /// File name of the picture, next to the manifest.
    pub image: String,
    pub dpi: u32,
    pub width_cm: u32,
    /// Seed that recreates this one case.
    pub seed: String,
    pub degraded: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Macro definitions from the paper a corpus case came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    /// arXiv id of the paper a corpus case came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Manifest {
    pub seed: u64,
    pub count: usize,
    pub made: usize,
    pub skipped: usize,
    pub cases: Vec<Case>,
}
