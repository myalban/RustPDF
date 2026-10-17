//! Update: reads a small JSON file `{"version": "x.y.z", "url": "…", "notes": "…"}` at the configured address.
//! The download goes through `curl` (shipped with Windows 10/11, macOS and most Linux distributions): no additional network dependency.

use crate::i18n::t;
use serde::Deserialize;
use std::process::Command;

#[derive(Deserialize, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub notes: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn parse_ver(v: &str) -> Vec<u32> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)).collect()
}

/// `true` if `candidate` is strictly more recent than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parse_ver(candidate), parse_ver(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

fn fetch(url: &str) -> Result<String, String> {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    c.args(["-sSL", "--max-time", "8", url]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Message to show to the user (new version available, up to date, or error).
pub fn check(url: &str) -> String {
    match fetch(url).and_then(|s| serde_json::from_str::<Manifest>(&s).map_err(|e| e.to_string())) {
        Ok(m) if is_newer(&m.version, current_version()) => format!("{} {} → {}\n{}\n{}", t("update_available"), current_version(), m.version, m.notes, m.url),
        Ok(_) => format!("{} ({})", t("update_up_to_date"), current_version()),
        Err(e) => format!("{}: {e}", t("update_error")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("0.3.0", "0.2.9"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(is_newer("0.2.0.1", "0.2.0"));
    }

    #[test]
    fn manifest_parses() {
        let m: Manifest = serde_json::from_str(r#"{"version":"1.2.3","url":"http://x"}"#).unwrap();
        assert_eq!(m.version, "1.2.3");
        assert!(m.notes.is_empty());
    }
}
