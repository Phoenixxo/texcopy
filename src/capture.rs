//! get an image of something on screen.

use anyhow::{Context, Result};
use image::DynamicImage;

/// capture one monitor, by index or the primary one.
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

/// crop to x,y,w,h, clamped to the image.
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

/// drag to select a region, None if cancelled.
pub fn interactive() -> Result<Option<DynamicImage>> {
    let path = std::env::temp_dir().join(format!("texcopy-{}.png", std::process::id()));
    std::fs::remove_file(&path).ok();
    let forced_overlay = std::env::var("TEXCOPY_SELECTOR").is_ok_and(|v| v == "overlay");
    let selected = if forced_overlay {
        None
    } else {
        system_selector(&path)?
    };
    let selected = match selected {
        Some(done) => done,
        None => overlay_selector(&path)?,
    };
    if !selected || !path.exists() {
        return Ok(None);
    }
    let img = image::open(&path);
    let _ = std::fs::remove_file(&path);
    Ok(Some(img?))
}

/// the OS's own selector, None if there isn't one.
#[cfg(target_os = "macos")]
fn system_selector(out: &std::path::Path) -> Result<Option<bool>> {
    use std::process::Command;
    // -i for crosshair selection, -x for no shutter sound.
    let status = Command::new("screencapture")
        .args(["-i", "-x"])
        .arg(out)
        .status()?;
    Ok(Some(status.success() && out.exists()))
}

#[cfg(target_os = "linux")]
fn system_selector(out: &std::path::Path) -> Result<Option<bool>> {
    let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    linux_tools::select(&dirs, wayland, out)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn system_selector(_out: &std::path::Path) -> Result<Option<bool>> {
    Ok(None)
}

/// our own overlay, run as a child process.
#[cfg(all(feature = "gui", feature = "capture"))]
fn overlay_selector(out: &std::path::Path) -> Result<bool> {
    use std::process::Command;
    let exe = std::env::current_exe().context("finding texcopy's own program")?;
    let output = Command::new(exe)
        .arg("--select-overlay")
        .arg(out)
        .output()
        .context("starting the selection overlay")?;
    if !output.status.success() {
        anyhow::bail!(
            "the selection overlay failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(out.exists())
}

#[cfg(not(all(feature = "gui", feature = "capture")))]
fn overlay_selector(_out: &std::path::Path) -> Result<bool> {
    anyhow::bail!(
        "drag-to-select isn't available in this build or on this OS. Use one of:\n  \
         texcopy --screen --region x,y,w,h   (capture the screen, crop to a pixel region)\n  \
         texcopy image.png                   (an image file)\n  \
         texcopy --clipboard                 (an image on the clipboard, e.g. from Snipping Tool or Flameshot)"
    )
}

/// region selector programs on linux.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // tests only, off linux.
mod linux_tools {
    use std::{
        path::{Path, PathBuf},
        process::Command,
    };

    use anyhow::Result;

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Tool {
        GrimSlurp,
        Maim,
        Scrot,
        Spectacle,
        GnomeScreenshot,
        Flameshot,
    }

    /// which one to try first on wayland or X11.
    fn order(wayland: bool) -> [Tool; 6] {
        use Tool::*;
        if wayland {
            [
                GrimSlurp,
                Spectacle,
                GnomeScreenshot,
                Flameshot,
                Maim,
                Scrot,
            ]
        } else {
            [
                Maim,
                Scrot,
                Flameshot,
                Spectacle,
                GnomeScreenshot,
                GrimSlurp,
            ]
        }
    }

    fn find(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
        dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
    }

    /// run the first selector that's installed.
    pub fn select(dirs: &[PathBuf], wayland: bool, out: &Path) -> Result<Option<bool>> {
        for tool in order(wayland) {
            if let Some(done) = run(tool, dirs, out)? {
                return Ok(Some(done));
            }
        }
        Ok(None)
    }

    /// None when the tool isn't installed.
    fn run(tool: Tool, dirs: &[PathBuf], out: &Path) -> Result<Option<bool>> {
        let wrote = |out: &Path| out.metadata().is_ok_and(|m| m.len() > 0);
        let status = |cmd: &mut Command| cmd.status().map(|s| s.success()).unwrap_or(false);
        let done = match tool {
            Tool::GrimSlurp => {
                let (Some(slurp), Some(grim)) = (find(dirs, "slurp"), find(dirs, "grim")) else {
                    return Ok(None);
                };
                // slurp prints the rectangle and exits non-zero on esc.
                let geometry = Command::new(slurp).output()?;
                if !geometry.status.success() {
                    return Ok(Some(false));
                }
                let geometry = String::from_utf8_lossy(&geometry.stdout).trim().to_string();
                status(Command::new(grim).args(["-g", &geometry]).arg(out)) && wrote(out)
            }
            Tool::Maim => {
                let Some(bin) = find(dirs, "maim") else {
                    return Ok(None);
                };
                status(Command::new(bin).arg("-s").arg(out)) && wrote(out)
            }
            Tool::Scrot => {
                let Some(bin) = find(dirs, "scrot") else {
                    return Ok(None);
                };
                status(Command::new(bin).arg("-s").arg(out)) && wrote(out)
            }
            Tool::Spectacle => {
                let Some(bin) = find(dirs, "spectacle") else {
                    return Ok(None);
                };
                // -b no window, -r region, -n no notification, -o output file.
                status(Command::new(bin).args(["-b", "-r", "-n", "-o"]).arg(out)) && wrote(out)
            }
            Tool::GnomeScreenshot => {
                let Some(bin) = find(dirs, "gnome-screenshot") else {
                    return Ok(None);
                };
                status(Command::new(bin).args(["-a", "-f"]).arg(out)) && wrote(out)
            }
            Tool::Flameshot => {
                let Some(bin) = find(dirs, "flameshot") else {
                    return Ok(None);
                };
                // --raw prints the png on stdout, nothing if cancelled.
                let shot = Command::new(bin).args(["gui", "--raw"]).output()?;
                if shot.status.success() && !shot.stdout.is_empty() {
                    std::fs::write(out, &shot.stdout)?;
                }
                wrote(out)
            }
        };
        Ok(Some(done))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn wayland_prefers_grim_and_x11_prefers_maim() {
            assert_eq!(order(true)[0], Tool::GrimSlurp);
            assert_eq!(order(false)[0], Tool::Maim);
            assert_eq!(order(true).len(), order(false).len());
        }

        #[cfg(unix)]
        fn script(dir: &Path, name: &str, body: &str) {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        #[cfg(unix)]
        #[test]
        fn the_first_installed_selector_runs_and_esc_means_cancelled() {
            let dir =
                std::env::temp_dir().join(format!("texcopy-tools-test-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let out = dir.join("out.png");
            let dirs = vec![dir.clone()];

            // nothing installed.
            assert_eq!(select(&dirs, false, &out).unwrap(), None);

            // maim writes the file named in its last argument.
            script(
                &dir,
                "maim",
                r#"for last; do :; done; echo PNGDATA > "$last""#,
            );
            assert_eq!(select(&dirs, false, &out).unwrap(), Some(true));
            assert!(out.exists());
            std::fs::remove_file(&out).unwrap();

            // esc exits without writing anything.
            script(&dir, "maim", "exit 1");
            assert_eq!(select(&dirs, false, &out).unwrap(), Some(false));

            // slurp's rectangle goes to grim and slurp failing means cancel.
            std::fs::remove_file(dir.join("maim")).unwrap();
            script(&dir, "slurp", r#"echo "10,20 300x200""#);
            script(
                &dir,
                "grim",
                r#"[ "$1" = "-g" ] && [ "$2" = "10,20 300x200" ] && echo PNG > "$3""#,
            );
            assert_eq!(select(&dirs, true, &out).unwrap(), Some(true));
            std::fs::remove_file(&out).unwrap();
            script(&dir, "slurp", "exit 1");
            assert_eq!(select(&dirs, true, &out).unwrap(), Some(false));
            assert!(!out.exists());

            // a missing tool gets skipped for the next one.
            std::fs::remove_file(dir.join("slurp")).unwrap();
            std::fs::remove_file(dir.join("grim")).unwrap();
            script(&dir, "scrot", r#"for last; do :; done; echo PNG > "$last""#);
            assert_eq!(select(&dirs, true, &out).unwrap(), Some(true));
            std::fs::remove_dir_all(&dir).ok();
        }
    }
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
