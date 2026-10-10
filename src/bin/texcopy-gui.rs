//! The same program as `texcopy`, built without a console window on Windows, so double-clicking it
//! opens just the window. (A console program shows a black terminal behind its window.) On other
//! systems it is identical to `texcopy`.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    texcopy::run()
}
