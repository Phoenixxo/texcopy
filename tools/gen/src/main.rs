//! texcopy-gen makes benchmark cases with real tools, see docs/BENCHMARKS.md.

mod corpus;
mod document;
mod formula;
mod generate;
mod manifest;
mod render;
mod rng;
mod util;
mod web;

use std::{collections::BTreeSet, path::PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Makes benchmark cases for texcopy", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make cases from a seed
    Generate {
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 100)]
        count: usize,
        /// Only these categories or prefixes: formula, matrix, cases, aligned, sentence, document, web, corpus
        #[arg(long, value_delimiter = ',')]
        kinds: Option<Vec<String>>,
        /// Output folder (default: tools/fixtures/generated/seedN-nN)
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 4)]
        jobs: usize,
    },
    /// Download CC-BY papers from arXiv for the corpus cases
    FetchArxiv {
        /// How many papers the corpus should hold
        #[arg(long, default_value_t = 12)]
        count: usize,
        #[arg(long, value_delimiter = ',', default_values_t = ["math.CO".to_string(), "cs.LG".to_string(), "math.PR".to_string()])]
        categories: Vec<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Generate {
            seed,
            count,
            kinds,
            out,
            jobs,
        } => {
            let kinds: Option<BTreeSet<String>> = kinds.map(|k| k.into_iter().collect());
            let dir = out.unwrap_or_else(|| generate::default_dir(seed, count, kinds.as_ref()));
            generate::generate(seed, count, &dir, kinds.as_ref(), jobs)?;
        }
        Command::FetchArxiv { count, categories } => {
            let n = corpus::fetch_arxiv(count, &categories)?;
            eprintln!(
                "corpus holds {n} papers in {}",
                corpus::corpus_dir().display()
            );
        }
    }
    Ok(())
}
