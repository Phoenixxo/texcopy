//! clipboard for every OS, on linux a detached copy of us keeps serving the text.

use anyhow::{Context, Result};
use image::DynamicImage;

pub fn read_image() -> Result<DynamicImage> {
    let img = arboard::Clipboard::new()?
        .get_image()
        .context("no image on the clipboard")?;
    let buf =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
            .context("malformed clipboard image")?;
    Ok(buf.into())
}

#[cfg(not(target_os = "linux"))]
pub fn copy_text(text: &str) -> Result<()> {
    arboard::Clipboard::new()?
        .set_text(text.to_owned())
        .context("writing to clipboard")
}

#[cfg(target_os = "linux")]
pub fn copy_text(text: &str) -> Result<()> {
    use std::{
        io::Write,
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        time::Duration,
    };
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--hold-clipboard")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0) // survive ctrl-c and the terminal closing.
        .spawn()
        .context("starting the clipboard helper")?;
    child
        .stdin
        .take()
        .context("helper stdin")?
        .write_all(text.as_bytes())?;

    // no display server means no clipboard, so exit.
    std::thread::sleep(Duration::from_millis(200));
    if let Some(status) = child.try_wait()? {
        anyhow::ensure!(
            status.success(),
            "no clipboard available (is a display server running?)"
        );
    }
    Ok(())
}

/// linux helper, serves stdin on the clipboard until something else takes over.
#[cfg(target_os = "linux")]
pub fn hold(text: &str) -> Result<()> {
    use arboard::SetExtLinux;
    arboard::Clipboard::new()?
        .set()
        .wait()
        .text(text.to_owned())?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn hold(_text: &str) -> Result<()> {
    anyhow::bail!("--hold-clipboard is only used on Linux")
}
