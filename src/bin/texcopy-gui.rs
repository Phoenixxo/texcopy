//! same as texcopy but without a console window on windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    texcopy::run()
}
