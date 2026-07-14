//! "Is there a newer release?" check against GitHub.
//!
//! Fail-silent by design: any network, HTTP, or parse error just means no nag —
//! never an error dialog, never a blocked launch. Exactly one request per launch,
//! on a background thread, polled from the UI loop like key recovery. This is the
//! only outbound network the editor makes, and it reads nothing local — just the
//! public releases endpoint. Naturally, it only helps users who are ALREADY on a
//! build that has it; older builds can't announce their own obsolescence.

use std::sync::mpsc::{self, Receiver};

const REPO: &str = "Deaththegrim/aincrad-save-editor";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A newer published release than the running build.
pub struct Update {
    /// The new version, without any leading `v` (e.g. "0.1.16").
    pub version: String,
    /// Where to get it.
    pub url: String,
}

/// Kick off the background check. Poll the returned receiver each frame; it
/// yields at most once, and only when a strictly-newer release exists (no
/// message at all otherwise — the thread just ends).
pub fn spawn_check() -> Receiver<Update> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Some(u) = check() {
            let _ = tx.send(u);
        }
    });
    rx
}

fn check() -> Option<Update> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    // GitHub rejects requests with no User-Agent; a short timeout keeps a slow or
    // captive network from parking the thread.
    let body = ureq::get(&url)
        .header("User-Agent", concat!("aincrad-save-editor/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    is_newer(tag, CURRENT).then(|| Update {
        version: tag.trim_start_matches('v').to_string(),
        url: format!("https://github.com/{REPO}/releases/latest"),
    })
}

/// Whether release tag `a` is a strictly-newer version than build `b`. Both may
/// carry a leading `v`. Unparseable input → false (stay quiet rather than nag).
fn is_newer(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Parse `[v]MAJOR.MINOR.PATCH[-suffix]` into a comparable triple. Missing minor
/// or patch default to 0; any trailing pre-release/build suffix is ignored.
fn parse(v: &str) -> Option<(u32, u32, u32)> {
    let core = v.trim().trim_start_matches('v');
    let mut nums = core.split(['.', '-', '+']).map(|p| p.parse::<u32>());
    let major = nums.next()?.ok()?;
    let minor = nums.next().and_then(Result::ok).unwrap_or(0);
    let patch = nums.next().and_then(Result::ok).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::{is_newer, parse};

    #[test]
    fn parses_with_and_without_v() {
        assert_eq!(parse("v0.1.15"), Some((0, 1, 15)));
        assert_eq!(parse("0.1.15"), Some((0, 1, 15)));
        assert_eq!(parse("1.2"), Some((1, 2, 0)));
        assert_eq!(parse("v0.2.0-beta.1"), Some((0, 2, 0)));
        assert_eq!(parse("nonsense"), None);
    }

    #[test]
    fn newer_only_when_strictly_greater() {
        assert!(is_newer("v0.1.16", "0.1.15"));
        assert!(is_newer("0.2.0", "0.1.15"));
        assert!(is_newer("v1.0.0", "0.9.99"));
        assert!(!is_newer("0.1.15", "0.1.15")); // same → no nag
        assert!(!is_newer("0.1.14", "0.1.15")); // older tag → no nag
        assert!(!is_newer("garbage", "0.1.15")); // unparseable → no nag
    }
}
