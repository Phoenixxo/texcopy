//! Locating the model files, and downloading them on first run.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::spec::ModelSpec;

fn cache_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Caches/texcopy"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".cache")))
            .map(|c| c.join("texcopy"))
    }
}

/// Find a directory holding a complete model and its spec, downloading the default
/// model into the cache if needed. Order: explicit override, `./models`, the per-user cache.
pub fn locate(over: Option<PathBuf>) -> Result<(PathBuf, ModelSpec)> {
    let cache = cache_dir();
    let candidates: Vec<PathBuf> = match &over {
        Some(d) => vec![d.clone()],
        None => [Some(PathBuf::from("models")), cache.clone()].into_iter().flatten().collect(),
    };
    for dir in candidates {
        let spec = ModelSpec::load(&dir)?;
        if spec.is_complete(&dir) {
            return Ok((dir, spec));
        }
    }

    // Nothing complete on disk: fetch into the override, or else the cache.
    let target = over
        .or(cache)
        .context("cannot determine a cache directory; pass --model-dir")?;
    let spec = ModelSpec::load(&target)?;
    let Some(base) = &spec.base_url else {
        bail!("model files for '{}' are missing from {} and its spec.json has no base_url", spec.name, target.display());
    };
    fs::create_dir_all(&target)?;
    eprintln!("First run: downloading model '{}' to {}", spec.name, target.display());
    for f in spec.files() {
        if !target.join(f).is_file() {
            download(&format!("{base}/{f}"), &target.join(f))?;
        }
    }
    Ok((target, spec))
}

fn download(url: &str, dest: &Path) -> Result<()> {
    let name = dest.file_name().unwrap().to_string_lossy().into_owned();
    let resp = ureq::get(url).call().with_context(|| format!("GET {url}"))?;
    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse().ok());
    let mut reader = resp.into_body().into_reader();

    // Write to a .part file so an interrupted download is never mistaken for a model.
    let part = dest.with_extension("part");
    let mut out = File::create(&part)?;
    let (mut buf, mut done, mut shown) = (vec![0u8; 1 << 16], 0u64, u64::MAX);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        // Redraw only when the whole-MB count changes.
        if done >> 20 != shown {
            shown = done >> 20;
            match total {
                Some(t) => eprint!("\r  {name}: {shown:>3}/{} MB", t >> 20),
                None => eprint!("\r  {name}: {shown} MB"),
            }
        }
    }
    eprintln!();
    fs::rename(&part, dest)?;
    Ok(())
}
