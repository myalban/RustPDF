//! Error log, data folder and crash report.

use std::io::Write;
use std::path::PathBuf;

/// Per-user configuration folder of the platform: `%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS, `$XDG_CONFIG_HOME` or `~/.config` on Linux.
fn config_base() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from).or_else(home)
    }
    #[cfg(target_os = "macos")]
    {
        home().map(|h| h.join("Library/Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home().map(|h| h.join(".config")))
    }
}

/// Application data folder: `<config>/RustPDF` (`<config>/rustpdf` on Linux, by convention).
pub fn data_dir() -> PathBuf {
    let base = config_base().or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))).unwrap_or_default();
    let d = base.join(if cfg!(all(unix, not(target_os = "macos"))) { "rustpdf" } else { "RustPDF" });
    let _ = std::fs::create_dir_all(&d);
    d
}

/// UTC date/time `YYYY-MM-DD HH:MM:SS` (without an external dependency).
pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's algorithm: days since 1970-01-01 → civil date.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, (rem % 3600) / 60, rem % 60)
}

pub fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn log_path() -> PathBuf {
    data_dir().join("rustpdf.log")
}

/// Appends a line to the log (rotation at 1 MB).
pub fn log(level: &str, msg: &str) {
    let p = log_path();
    if std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} [{level}] {msg}", timestamp());
    }
}

pub fn info(msg: impl AsRef<str>) {
    log("INFO", msg.as_ref());
}
pub fn warn(msg: impl AsRef<str>) {
    log("WARN", msg.as_ref());
}
pub fn error(msg: impl AsRef<str>) {
    log("ERROR", msg.as_ref());
}

/// Installs a panic handler that writes a report to the data folder.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let report = format!(
            "RustPDF {} — crash report\n{}\n\n{info}\n\n{bt}\n",
            env!("CARGO_PKG_VERSION"),
            timestamp()
        );
        let dir = data_dir().join("crash");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("crash-{}.txt", unix_secs()));
        let _ = std::fs::write(&file, &report);
        error(format!("PANIC: {info} (report: {})", file.display()));
        default(info);
    }));
}

/// Existing crash reports (most recent first).
pub fn crash_reports() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(data_dir().join("crash"))
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_looks_valid() {
        let t = timestamp();
        assert_eq!(t.len(), 19, "{t}");
        assert!(t.starts_with("20"), "{t}");
    }
}
