//! Updates: asks GitHub for the latest release (or any JSON file `{"version","url","notes"}` set in the
//! preferences) and tells the user when a newer version exists. Nothing is downloaded or installed silently:
//! the user opens the release page and takes the installer for their system.
//! The request goes through `curl` (shipped with Windows 10/11, macOS and most Linux distributions): no additional network dependency.

use crate::i18n::t;
use serde::Deserialize;
use std::process::Command;

/// Where the latest release is looked up when the preferences do not give another address.
pub const DEFAULT_URL: &str = "https://api.github.com/repos/myalban/rustpdf/releases/latest";

/// A published release.
#[derive(Debug, PartialEq, Clone)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub notes: String,
}

/// Fields of both accepted formats: the GitHub API (`tag_name`, `html_url`, `body`) and the custom file (`version`, `url`, `notes`).
#[derive(Deserialize, Default)]
#[serde(default)]
struct Raw {
    tag_name: String,
    html_url: String,
    body: String,
    version: String,
    url: String,
    notes: String,
}

pub enum Outcome {
    Newer(Release),
    UpToDate,
    Error(String),
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

/// Reads a release from the GitHub API answer or from the custom JSON file.
pub fn parse(json: &str) -> Result<Release, String> {
    let r: Raw = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let version = if r.tag_name.is_empty() { r.version } else { r.tag_name }.trim().trim_start_matches('v').to_string();
    if version.is_empty() {
        return Err("no version in the answer".into());
    }
    let url = if r.html_url.is_empty() { r.url } else { r.html_url };
    let notes = if r.body.is_empty() { r.notes } else { r.body };
    Ok(Release { version, url, notes })
}

/// Address to query: the one from the preferences, or GitHub.
pub fn effective_url(pref: &str) -> &str {
    if pref.trim().is_empty() { DEFAULT_URL } else { pref.trim() }
}

fn fetch(url: &str) -> Result<String, String> {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    // GitHub's API refuses requests without a User-Agent
    c.args(["-sSL", "--max-time", "8", "-H", "User-Agent: RustPDF", "-H", "Accept: application/vnd.github+json", url]);
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

/// Queries the address and compares with the running version.
pub fn check_release(url: &str) -> Outcome {
    match fetch(url).and_then(|s| parse(&s)) {
        Ok(r) if is_newer(&r.version, current_version()) => Outcome::Newer(r),
        Ok(_) => Outcome::UpToDate,
        Err(e) => Outcome::Error(e),
    }
}

/// Message to show to the user. A line starting with `http` becomes a clickable link in the dialog.
pub fn describe(o: &Outcome) -> String {
    match o {
        Outcome::Newer(r) => format!("{} {} → {}\n{}\n{}", t("update_available"), current_version(), r.version, r.url, r.notes.trim()),
        Outcome::UpToDate => format!("{} ({})", t("update_up_to_date"), current_version()),
        Outcome::Error(e) => format!("{}: {e}", t("update_error")),
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
        assert!(is_newer("v0.10.0", "0.9.0"));
    }

    #[test]
    fn custom_manifest_parses() {
        let r = parse(r#"{"version":"1.2.3","url":"http://x"}"#).unwrap();
        assert_eq!(r, Release { version: "1.2.3".into(), url: "http://x".into(), notes: String::new() });
    }

    #[test]
    fn github_release_parses() {
        let json = r#"{"tag_name":"v0.3.0","html_url":"https://github.com/myalban/rustpdf/releases/tag/v0.3.0","body":"What is new","assets":[{"name":"x"}],"draft":false}"#;
        let r = parse(json).unwrap();
        assert_eq!(r.version, "0.3.0");
        assert!(r.url.ends_with("/v0.3.0"));
        assert_eq!(r.notes, "What is new");
    }

    #[test]
    fn answers_without_a_version_are_errors() {
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn empty_preference_means_github() {
        assert_eq!(effective_url("  "), DEFAULT_URL);
        assert_eq!(effective_url("https://example.org/latest.json"), "https://example.org/latest.json");
    }
}
