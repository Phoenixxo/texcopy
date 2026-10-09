//! Terminal user interface: a yes/no prompt and download progress.
//!
//! Everything goes to stderr, so stdout stays clean for the LaTeX (`texcopy x.png > out.tex`).

use std::{
    io::{BufRead, IsTerminal, Write},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};

/// Is a person sitting at a terminal (so a prompt makes sense)?
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Ask a yes/no question. Default is no. `assume_yes` (--yes) answers for the user. When nobody can
/// answer (a script, a pipe, --json) this is an error that says how to approve, never a silent yes.
pub fn confirm(question: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    if !interactive() {
        bail!(
            "this needs your approval, and there is no terminal to ask in. Run `texcopy --setup` once in a terminal, or add --yes to approve."
        );
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// 1234567 -> "1.2 MB" (decimal units, like download pages show).
pub fn human_bytes(n: u64) -> String {
    let n = n as f64;
    if n < 1e3 {
        format!("{n:.0} B")
    } else if n < 1e6 {
        format!("{:.1} kB", n / 1e3)
    } else if n < 1e9 {
        format!("{:.1} MB", n / 1e6)
    } else {
        format!("{:.2} GB", n / 1e9)
    }
}

pub fn human_duration(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// One progress line, e.g.
/// `  model.gguf             [████████████░░░░░░░░░░░░]  50%  406.8 MB / 813.6 MB  12.0 MB/s  ETA 34s`
pub fn bar_line(label: &str, done: u64, total: Option<u64>, bytes_per_sec: f64) -> String {
    const WIDTH: usize = 24;
    let label: String = label.chars().take(22).collect();
    let speed = if bytes_per_sec > 0.0 {
        format!("{}/s", human_bytes(bytes_per_sec as u64))
    } else {
        "...".to_string()
    };
    match total.filter(|&t| t > 0) {
        Some(total) => {
            let frac = (done as f64 / total as f64).clamp(0.0, 1.0);
            let filled = (frac * WIDTH as f64).round() as usize;
            let eta = if bytes_per_sec > 0.0 {
                human_duration((total.saturating_sub(done)) as f64 / bytes_per_sec)
            } else {
                "?".into()
            };
            format!(
                "  {label:<22} [{}{}] {:>3.0}%  {} / {}  {speed}  ETA {eta}",
                "█".repeat(filled),
                "░".repeat(WIDTH - filled),
                frac * 100.0,
                human_bytes(done),
                human_bytes(total),
            )
        }
        None => format!("  {label:<22} {}  {speed}", human_bytes(done)),
    }
}

/// Download progress: a live bar on a terminal, a line per 10% in a log.
pub struct Progress {
    label: String,
    total: Option<u64>,
    /// bytes already on disk when this run started (a resumed download)
    base: u64,
    start: Instant,
    last_draw: Instant,
    tty: bool,
    next_pct: u64,
}

impl Progress {
    pub fn new(label: &str, total: Option<u64>, base: u64) -> Self {
        let now = Instant::now();
        Progress {
            label: label.to_string(),
            total,
            base,
            start: now,
            last_draw: now - Duration::from_secs(1),
            tty: std::io::stderr().is_terminal(),
            next_pct: 0,
        }
    }

    fn speed(&self, done: u64) -> f64 {
        let secs = self.start.elapsed().as_secs_f64();
        if secs < 0.25 {
            0.0
        } else {
            done.saturating_sub(self.base) as f64 / secs
        }
    }

    pub fn update(&mut self, done: u64) {
        if self.tty {
            if self.last_draw.elapsed() >= Duration::from_millis(100) {
                self.last_draw = Instant::now();
                eprint!(
                    "\r\x1b[K{}",
                    bar_line(&self.label, done, self.total, self.speed(done))
                );
                std::io::stderr().flush().ok();
            }
        } else if let Some(total) = self.total.filter(|&t| t > 0) {
            let pct = done * 100 / total;
            if pct >= self.next_pct {
                eprintln!(
                    "{}",
                    bar_line(&self.label, done, self.total, self.speed(done))
                );
                self.next_pct = (pct / 10 + 1) * 10;
            }
        }
    }

    pub fn finish(&mut self, done: u64) {
        let line = bar_line(&self.label, done, self.total, self.speed(done));
        if self.tty {
            eprintln!("\r\x1b[K{line}");
        } else {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bar_line, confirm, human_bytes, human_duration};

    #[test]
    fn sizes_and_durations_read_naturally() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(11_189_714), "11.2 MB");
        assert_eq!(human_bytes(813_576_320), "813.6 MB");
        assert_eq!(human_bytes(1_018_563_360), "1.02 GB");
        assert_eq!(human_duration(5.2), "5s");
        assert_eq!(human_duration(75.0), "1m15s");
        assert_eq!(human_duration(3700.0), "1h01m");
    }

    #[test]
    fn progress_line_shows_percent_size_speed_and_eta() {
        let line = bar_line("model.gguf", 50_000_000, Some(100_000_000), 10_000_000.0);
        assert!(line.contains(" 50%"), "{line}");
        assert!(line.contains("50.0 MB / 100.0 MB"), "{line}");
        assert!(line.contains("10.0 MB/s"), "{line}");
        assert!(line.contains("ETA 5s"), "{line}");
        assert_eq!(line.matches('█').count(), 12);
        // unknown size, no speed yet: still a readable line
        assert!(bar_line("x", 1_500_000, None, 0.0).contains("1.5 MB"));
        // overshoot never panics or draws more than the bar
        assert!(bar_line("x", 200, Some(100), 1.0).contains("100%"));
    }

    #[test]
    fn approval_is_never_assumed_without_a_terminal() {
        // `cargo test` has no terminal on stdin: asking must fail loudly, and --yes must work.
        assert!(confirm("Install?", true).unwrap());
        if !super::interactive() {
            let err = confirm("Install?", false).unwrap_err().to_string();
            assert!(err.contains("--setup") && err.contains("--yes"), "{err}");
        }
    }
}
