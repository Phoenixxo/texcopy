//! Getting an image of something on screen.
//!
//! `screenshot` works on Windows, macOS and Linux (via the `xcap` crate) and returns
//! a whole monitor; cropping to a region is done by the caller. `interactive` is the
//! drag-to-select experience, which currently only exists where the OS provides it.

use anyhow::{Context, Result};
use image::DynamicImage;

/// Capture one monitor: `index` into the OS's monitor list, or the primary monitor.
#[cfg(feature = "capture")]
pub fn screenshot(index: Option<usize>) -> Result<DynamicImage> {
    let monitors = xcap::Monitor::all().context("listing monitors")?;
    let monitor = match index {
        Some(i) => monitors.into_iter().nth(i).with_context(|| {
            format!(
                "no monitor {i}; use 0..{}",
                xcap::Monitor::all()
                    .map_or(0, |m| m.len())
                    .saturating_sub(1)
            )
        })?,
        None => {
            let primary = monitors
                .iter()
                .position(|m| m.is_primary().unwrap_or(false));
            monitors
                .into_iter()
                .nth(primary.unwrap_or(0))
                .context("no monitors found")?
        }
    };
    let img = monitor.capture_image().context("capturing the screen")?;
    Ok(DynamicImage::ImageRgba8(img))
}

#[cfg(not(feature = "capture"))]
pub fn screenshot(_index: Option<usize>) -> Result<DynamicImage> {
    anyhow::bail!("this build has no screen capture; rebuild with the default `capture` feature")
}

/// Crop `img` to `x,y,w,h` (pixels in the captured image), clamped to its bounds.
pub fn crop(img: &DynamicImage, region: &str) -> Result<DynamicImage> {
    let nums: Vec<u32> = region
        .split(',')
        .map(|n| n.trim().parse())
        .collect::<Result<_, _>>()
        .with_context(|| format!("--region expects x,y,w,h (whole numbers), got '{region}'"))?;
    let [x, y, w, h] = nums[..] else {
        anyhow::bail!("--region expects exactly four numbers: x,y,w,h");
    };
    anyhow::ensure!(
        x < img.width() && y < img.height(),
        "--region starts outside the {}x{} screenshot",
        img.width(),
        img.height()
    );
    let (w, h) = (w.min(img.width() - x), h.min(img.height() - y));
    anyhow::ensure!(w > 0 && h > 0, "--region has zero width or height");
    Ok(img.crop_imm(x, y, w, h))
}

/// Drag-select a region with the OS's own tool. `None` means the user cancelled.
#[cfg(target_os = "macos")]
pub fn interactive() -> Result<Option<DynamicImage>> {
    use std::process::Command;
    let path = std::env::temp_dir().join(format!("texcopy-{}.png", std::process::id()));
    // -i: interactive crosshair selection, -x: no shutter sound. Esc cancels (no file written).
    let status = Command::new("screencapture")
        .args(["-i", "-x"])
        .arg(&path)
        .status()?;
    if !status.success() || !path.exists() {
        return Ok(None);
    }
    let img = image::open(&path);
    let _ = std::fs::remove_file(&path);
    Ok(Some(img?))
}

#[cfg(not(target_os = "macos"))]
pub fn interactive() -> Result<Option<DynamicImage>> {
    anyhow::bail!(
        "drag-to-select isn't built in on this OS yet. Use one of:\n  \
         texcopy --screen --region x,y,w,h   (capture the screen, crop to a pixel region)\n  \
         texcopy image.png                   (an image file)\n  \
         texcopy --clipboard                 (an image on the clipboard, e.g. from Snipping Tool or Flameshot)"
    )
}

#[cfg(test)]
mod tests {
    use super::crop;
    use image::DynamicImage;

    fn img() -> DynamicImage {
        DynamicImage::new_rgb8(100, 50)
    }

    #[test]
    fn crops_and_clamps() {
        let c = crop(&img(), "10, 5, 20, 10").unwrap();
        assert_eq!((c.width(), c.height()), (20, 10));
        let c = crop(&img(), "90,40,500,500").unwrap();
        assert_eq!((c.width(), c.height()), (10, 10));
    }

    #[test]
    fn rejects_bad_regions() {
        assert!(crop(&img(), "1,2,3").is_err());
        assert!(crop(&img(), "a,b,c,d").is_err());
        assert!(crop(&img(), "200,0,10,10").is_err());
        assert!(crop(&img(), "0,0,0,10").is_err());
    }
}
