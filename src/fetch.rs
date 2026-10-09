//! Locating the model files, and downloading them on first run.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::{spec::ModelSpec, ui};

/// Per-user cache directory for downloaded files (platform-appropriate on every OS).
/// `TEXCOPY_CACHE_DIR` overrides it, which is handy for tests and portable installs.
pub(crate) fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TEXCOPY_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "texcopy").map(|d| d.cache_dir().to_path_buf())
}

/// Find a directory holding a complete model and its spec, downloading the default
/// model into the cache if needed. Order: explicit override, `./models`, the per-user cache.
pub fn locate(over: Option<PathBuf>) -> Result<(PathBuf, ModelSpec)> {
    let cache = cache_dir();
    let candidates: Vec<PathBuf> = match &over {
        Some(d) => vec![d.clone()],
        None => [Some(PathBuf::from("models")), cache.clone()]
            .into_iter()
            .flatten()
            .collect(),
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
        bail!(
            "model files for '{}' are missing from {} and its spec.json has no base_url",
            spec.name,
            target.display()
        );
    };
    fs::create_dir_all(&target)?;
    eprintln!(
        "First run: downloading model '{}' to {}",
        spec.name,
        target.display()
    );
    for f in spec.files() {
        if !target.join(f).is_file() {
            download(&format!("{base}/{f}"), &target.join(f))?;
        }
    }
    Ok((target, spec))
}

/// A file to fetch. `size` and `sha256` are checked when given.
pub(crate) struct Download<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub label: &'a str,
    pub size: Option<u64>,
    pub sha256: Option<&'a str>,
}

/// Download without checks (used for the small default model).
pub(crate) fn download(url: &str, dest: &Path) -> Result<()> {
    let label = dest
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    download_verified(&Download {
        url,
        dest,
        label: &label,
        size: None,
        sha256: None,
    })
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let (mut hasher, mut buf) = (Sha256::new(), vec![0u8; 1 << 20]);
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Download with a progress display, resuming an interrupted `.part` file, and verifying the size
/// and SHA-256 when they are known. A file that fails verification is deleted, never kept.
pub(crate) fn download_verified(d: &Download) -> Result<()> {
    if d.dest.is_file()
        && d.sha256
            .is_none_or(|h| sha256_file(d.dest).is_ok_and(|got| got == h))
    {
        return Ok(()); // already here (and intact, when we can tell)
    }
    // Write to a .part file so an interrupted download is never mistaken for the real thing.
    let mut part = d.dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);

    let mut have = fs::metadata(&part).map_or(0, |m| m.len());
    if d.size.is_some_and(|total| have > total) {
        fs::remove_file(&part).ok();
        have = 0;
    }
    if d.size != Some(have) {
        let mut req = ureq::get(d.url);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let resp = req
            .call()
            .with_context(|| format!("downloading {}", d.url))?;
        // 206 means the server continued where we stopped; anything else sends the whole file.
        let resumed = have > 0 && resp.status().as_u16() == 206;
        if !resumed {
            have = 0;
        }
        let remaining: Option<u64> = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()?.parse().ok());
        let total = d.size.or(remaining.map(|r| r + have));
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&part)?;
        if resumed {
            eprintln!("  resuming {} at {}", d.label, ui::human_bytes(have));
        }
        let mut reader = resp.into_body().into_reader();
        let mut progress = ui::Progress::new(d.label, total, have);
        let (mut buf, mut done) = (vec![0u8; 1 << 16], have);
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress.update(done);
        }
        out.flush()?;
        progress.finish(done);
    }

    let got = fs::metadata(&part)?.len();
    if let Some(want) = d.size.filter(|&w| w != got) {
        bail!(
            "{} is incomplete ({} of {}). Run the command again to resume.",
            d.label,
            ui::human_bytes(got),
            ui::human_bytes(want)
        );
    }
    if let Some(want) = d.sha256 {
        eprintln!("  verifying {} ...", d.label);
        let digest = sha256_file(&part)?;
        if digest != want {
            fs::remove_file(&part).ok();
            bail!(
                "{} failed its checksum (got {digest}, expected {want}). The download was discarded; try again.",
                d.label
            );
        }
    }
    fs::rename(&part, d.dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Download, download_verified, sha256_file};
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
    };

    /// Serve `data` over HTTP on localhost for `connections` requests. With `honour_range`, a
    /// `Range: bytes=N-` request gets a 206 continuing at N, like Hugging Face and GitHub do.
    fn serve(data: Vec<u8>, honour_range: bool, connections: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming().take(connections) {
                let mut stream = stream.unwrap();
                let mut req = Vec::new();
                let mut b = [0u8; 1];
                while !req.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut b).unwrap() == 0 {
                        break;
                    }
                    req.push(b[0]);
                }
                let req = String::from_utf8_lossy(&req).to_lowercase();
                let start = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
                let reply = match start {
                    Some(n) if honour_range => format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {n}-{}/{}\r\nConnection: close\r\n\r\n",
                        data.len() - n,
                        data.len() - 1,
                        data.len()
                    ),
                    _ => format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    ),
                };
                let body = match start {
                    Some(n) if honour_range => &data[n..],
                    _ => &data[..],
                };
                stream.write_all(reply.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        url
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("texcopy-dl-{}-{name}", std::process::id()));
        fs::remove_dir_all(&d).ok();
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn payload() -> (Vec<u8>, String) {
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let sha = Sha256::digest(&data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        (data, sha)
    }

    fn fetch(url: &str, dir: &std::path::Path, size: u64, sha: &str) -> anyhow::Result<()> {
        download_verified(&Download {
            url,
            dest: &dir.join("file.bin"),
            label: "test file",
            size: Some(size),
            sha256: Some(sha),
        })
    }

    #[test]
    fn downloads_and_verifies() {
        let (data, sha) = payload();
        let dir = tmp("ok");
        fetch(&serve(data.clone(), true, 1), &dir, 5000, &sha).unwrap();
        assert_eq!(fs::read(dir.join("file.bin")).unwrap(), data);
        assert_eq!(sha256_file(&dir.join("file.bin")).unwrap(), sha);
        assert!(!dir.join("file.bin.part").exists());
    }

    #[test]
    fn a_wrong_checksum_is_rejected_and_nothing_is_kept() {
        let (data, _) = payload();
        let dir = tmp("badsum");
        let err = fetch(&serve(data, true, 1), &dir, 5000, &"0".repeat(64))
            .unwrap_err()
            .to_string();
        assert!(err.contains("checksum"), "{err}");
        assert!(
            !dir.join("file.bin").exists(),
            "a bad file must never be installed"
        );
        assert!(
            !dir.join("file.bin.part").exists(),
            "and the bad partial is deleted"
        );
    }

    #[test]
    fn an_interrupted_download_resumes_where_it_stopped() {
        let (data, sha) = payload();
        let dir = tmp("resume");
        fs::write(dir.join("file.bin.part"), &data[..1200]).unwrap();
        fetch(&serve(data.clone(), true, 1), &dir, 5000, &sha).unwrap();
        assert_eq!(fs::read(dir.join("file.bin")).unwrap(), data);
    }

    #[test]
    fn a_server_that_ignores_range_just_restarts_cleanly() {
        let (data, sha) = payload();
        let dir = tmp("norange");
        fs::write(dir.join("file.bin.part"), &data[..1200]).unwrap();
        fetch(&serve(data.clone(), false, 1), &dir, 5000, &sha).unwrap();
        assert_eq!(fs::read(dir.join("file.bin")).unwrap(), data);
    }

    #[test]
    fn a_file_that_is_already_intact_is_not_downloaded_again() {
        let (data, sha) = payload();
        let dir = tmp("have");
        fs::write(dir.join("file.bin"), &data).unwrap();
        // no server at all: it must not even try
        fetch("http://127.0.0.1:9/never", &dir, 5000, &sha).unwrap();
    }

    #[test]
    fn cache_dir_is_per_user_and_named_texcopy() {
        let dir = super::cache_dir().expect("a cache directory");
        assert_eq!(dir.file_name().unwrap(), "texcopy", "{dir:?}");
        assert!(dir.is_absolute(), "{dir:?}");
    }
}
