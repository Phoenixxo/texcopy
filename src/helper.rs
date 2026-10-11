//! installs llama-server for the user, pinned release and checksum checked.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    fetch::{self, Download},
    ui::{Event, Reporter},
};

pub const TAG: &str = "b11146";
const DOWNLOAD_BASE: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    TarGz,
    Zip,
}

#[derive(Debug)]
pub struct Release {
    pub os: &'static str,
    pub arch: &'static str,
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub archive: Archive,
    /// what this build is, for the prompt.
    pub describe: &'static str,
}

/// CPU builds per platform, macOS arm64 has Metal.
pub const RELEASES: &[Release] = &[
    Release {
        os: "macos",
        arch: "aarch64",
        name: "llama-b11146-bin-macos-arm64.tar.gz",
        size: 11_189_714,
        sha256: "1ad3f9eff80edb9dbef4259ad564d1720612ef7eea48fa4afed0e54f5f3d5711",
        archive: Archive::TarGz,
        describe: "macOS (Apple Silicon, uses the GPU)",
    },
    Release {
        os: "macos",
        arch: "x86_64",
        name: "llama-b11146-bin-macos-x64.tar.gz",
        size: 11_237_237,
        sha256: "305f0e3a17d2c01eb205cd0a62128357f1ec3b55329cb084d94e5ec0115d7a3b",
        archive: Archive::TarGz,
        describe: "macOS (Intel)",
    },
    Release {
        os: "linux",
        arch: "x86_64",
        name: "llama-b11146-bin-ubuntu-x64.tar.gz",
        size: 16_998_357,
        sha256: "c150306eb16b5ab696f76a8bdf810c35fd98a24e82158742e6fa28f420ff8410",
        archive: Archive::TarGz,
        describe: "Linux x64 (CPU build)",
    },
    Release {
        os: "linux",
        arch: "aarch64",
        name: "llama-b11146-bin-ubuntu-arm64.tar.gz",
        size: 13_598_346,
        sha256: "4aeda6fe68831547e49b7fa87607383ca5352b3d72ca5f70d52ed265f58c131f",
        archive: Archive::TarGz,
        describe: "Linux arm64 (CPU build)",
    },
    Release {
        os: "windows",
        arch: "x86_64",
        name: "llama-b11146-bin-win-cpu-x64.zip",
        size: 18_560_055,
        sha256: "14cf1303ca9ac3abd94816850532f9f9a69ac66fbaca3776fc6f9061c2fac1d1",
        archive: Archive::Zip,
        describe: "Windows x64 (CPU build)",
    },
    Release {
        os: "windows",
        arch: "aarch64",
        name: "llama-b11146-bin-win-cpu-arm64.zip",
        size: 12_034_624,
        sha256: "1727d241f3bf6d27360e984e851cf013928fd655bf89f8628e70da027f377b7d",
        archive: Archive::Zip,
        describe: "Windows arm64 (CPU build)",
    },
];

pub fn release_for(os: &str, arch: &str) -> Option<&'static Release> {
    RELEASES.iter().find(|r| r.os == os && r.arch == arch)
}

/// the build for this machine if llama.cpp has one.
pub fn release_for_this_machine() -> Option<&'static Release> {
    release_for(std::env::consts::OS, std::env::consts::ARCH)
}

impl Release {
    pub fn url(&self) -> String {
        format!("{DOWNLOAD_BASE}/{TAG}/{}", self.name)
    }
}

/// where texcopy keeps the helper it installed.
pub fn install_dir() -> Option<PathBuf> {
    fetch::cache_dir().map(|d| d.join("llama.cpp").join(TAG))
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// llama-server somewhere under dir.
pub fn find_binary(dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_binary(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == binary_name()) {
            return Some(path);
        }
    }
    None
}

/// the helper installed earlier, if any.
pub fn installed() -> Option<PathBuf> {
    find_binary(&install_dir()?)
}

/// download, verify and unpack the helper.
pub fn install(release: &Release, report: Reporter) -> Result<PathBuf> {
    let dir = install_dir().context("cannot determine a cache directory")?;
    if let Some(bin) = find_binary(&dir) {
        return Ok(bin);
    }
    let parent = dir.parent().context("bad install directory")?;
    fs::create_dir_all(parent)?;
    let archive = parent.join(release.name);
    fetch::download_verified(
        &Download {
            url: &release.url(),
            dest: &archive,
            label: &format!("llama.cpp {TAG}"),
            size: Some(release.size),
            sha256: Some(release.sha256),
        },
        report,
    )?;

    // unpack beside the final folder then move it in, so a crash never leaves half an install.
    let staging = parent.join(format!("{TAG}.unpacking"));
    fs::remove_dir_all(&staging).ok();
    fs::create_dir_all(&staging)?;
    report(Event::Unpacking);
    match release.archive {
        Archive::TarGz => unpack_tar_gz(&archive, &staging)?,
        Archive::Zip => unpack_zip(&archive, &staging)?,
    }
    let bin = find_binary(&staging)
        .with_context(|| format!("{} is not in {}", binary_name(), release.name))?;
    make_executable(&bin)?;
    fs::remove_dir_all(&dir).ok();
    fs::rename(&staging, &dir)?;
    fs::remove_file(&archive).ok();
    find_binary(&dir).context("the installed helper vanished")
}

pub fn unpack_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let file = File::open(archive)?;
    tar::Archive::new(flate2::read::GzDecoder::new(file))
        .unpack(dest)
        .with_context(|| format!("unpacking {}", archive.display()))
}

pub fn unpack_zip(archive: &Path, dest: &Path) -> Result<()> {
    zip::ZipArchive::new(File::open(archive)?)?
        .extract(dest)
        .with_context(|| format!("unpacking {}", archive.display()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    if perms.mode() & 0o111 == 0 {
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// stop on a platform with no published build.
pub fn require_release() -> Result<&'static Release> {
    release_for_this_machine().with_context(|| {
        format!(
            "llama.cpp publishes no build for {} {}; {}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            crate::vlm::INSTALL_HINT
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn every_platform_we_support_has_a_pinned_build() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("windows", "x86_64"),
        ] {
            let r = release_for(os, arch).unwrap_or_else(|| panic!("{os} {arch}"));
            assert_eq!(r.sha256.len(), 64, "{}", r.name);
            assert!(r.name.contains(TAG), "{}", r.name);
            assert!(
                r.url()
                    .starts_with("https://github.com/ggml-org/llama.cpp/releases/download/b11146/")
            );
        }
        assert!(release_for("freebsd", "x86_64").is_none());
        assert_eq!(
            release_for("windows", "x86_64").unwrap().archive,
            Archive::Zip
        );
        assert_eq!(
            release_for("macos", "aarch64").unwrap().archive,
            Archive::TarGz
        );
        assert!(
            RELEASES.iter().all(|r| r.name.contains(TAG)),
            "every pinned file belongs to {TAG}"
        );
    }

    #[test]
    fn unpacks_tar_gz_and_zip_and_finds_the_server_inside_a_folder() {
        let tmp = std::env::temp_dir().join(format!("texcopy-helper-test-{}", std::process::id()));
        fs::remove_dir_all(&tmp).ok();
        fs::create_dir_all(&tmp).unwrap();

        // .tar.gz with the binary one folder down.
        let tgz = tmp.join("a.tar.gz");
        {
            let enc = flate2::write::GzEncoder::new(
                File::create(&tgz).unwrap(),
                flate2::Compression::fast(),
            );
            let mut tarball = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            let body = b"#!/bin/sh\necho hi\n";
            header.set_size(body.len() as u64);
            header.set_mode(0o644); // not executable on purpose, install has to fix that.
            header.set_cksum();
            tarball
                .append_data(
                    &mut header,
                    format!("llama-{TAG}/{}", binary_name()),
                    &body[..],
                )
                .unwrap();
            tarball.into_inner().unwrap().finish().unwrap();
        }
        let out1 = tmp.join("tgz");
        fs::create_dir_all(&out1).unwrap();
        unpack_tar_gz(&tgz, &out1).unwrap();
        let bin = find_binary(&out1).expect("binary found");
        assert!(bin.ends_with(binary_name()));
        make_executable(&bin).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(fs::metadata(&bin).unwrap().permissions().mode() & 0o111, 0);
        }

        // .zip
        let zp = tmp.join("a.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&zp).unwrap());
            z.start_file(binary_name(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"MZ").unwrap();
            z.finish().unwrap();
        }
        let out2 = tmp.join("zip");
        fs::create_dir_all(&out2).unwrap();
        unpack_zip(&zp, &out2).unwrap();
        assert!(find_binary(&out2).is_some());

        assert!(find_binary(&tmp.join("missing")).is_none());
        fs::remove_dir_all(&tmp).ok();
    }
}
