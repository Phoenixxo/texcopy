//! Clipboard access that behaves the same on Windows, macOS and Linux.
//!
//! One catch: on X11 and Wayland the clipboard contents are served by the program that
//! set them, so a command-line tool that copies and exits would leave the clipboard
//! empty. On Linux we therefore hand the text to a detached copy of ourselves
//! (`--hold-clipboard`) that keeps serving it until another program takes over.

use anyhow::{Context, Result};
use image::DynamicImage;

pub fn read_image() -> Result<DynamicImage> {
    let img = arboard::Clipboard::new()?.get_image().context("no image on the clipboard")?;
    let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
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
        .process_group(0) // survive Ctrl-C and the terminal closing
        .spawn()
        .context("starting the clipboard helper")?;
    child.stdin.take().context("helper stdin")?.write_all(text.as_bytes())?;

    // If the helper can't reach a clipboard (no display server), it exits immediately.
    std::thread::sleep(Duration::from_millis(200));
    if let Some(status) = child.try_wait()? {
        anyhow::ensure!(status.success(), "no clipboard available (is a display server running?)");
    }
    Ok(())
}

/// Body of the Linux helper process: read text from stdin and serve it until overwritten.
#[cfg(target_os = "linux")]
pub fn hold(text: &str) -> Result<()> {
    use arboard::SetExtLinux;
    arboard::Clipboard::new()?.set().wait().text(text.to_owned())?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn hold(_text: &str) -> Result<()> {
    anyhow::bail!("--hold-clipboard is only used on Linux")
}
