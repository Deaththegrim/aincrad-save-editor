//! `nxm://` protocol handling — the "Mod Manager Download" button on Nexus Mods.
//!
//! Clicking that button hands the OS a URL like
//! `nxm://<game>/mods/<mod_id>/files/<file_id>?key=<k>&expires=<t>&user_id=<u>`.
//! We register aml as the handler for the `nxm` scheme, parse that URL, exchange
//! it for a CDN link via the Nexus API, download the file, and stage it in the
//! library.
//!
//! Verified formats (Nexus Mods API docs + node-nexus-api):
//! - URL: `nxm://GAME/mods/MOD/files/FILE?key=…&expires=…` (key/expires are
//!   present for free users; premium keys can omit them).
//! - API: `GET https://api.nexusmods.com/v1/games/GAME/mods/MOD/files/FILE/download_link.json`
//!   with the personal API key in the `apikey` header; the response is a JSON
//!   array of `{ name, short_name, URI }` CDN links.
//!
//! The network exchange shells out to `curl` (same "wrap a proven CLI" strategy
//! as the pak backends), so aml stays free of a bundled TLS stack.

use crate::HostError;
use std::path::{Path, PathBuf};

/// A parsed `nxm://` download request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NxmUrl {
    /// Nexus "game domain", e.g. `echoesofaincrad`.
    pub game: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// Time-limited download token (free users); `None` for premium.
    pub key: Option<String>,
    /// Expiry timestamp paired with `key`.
    pub expires: Option<String>,
}

impl NxmUrl {
    /// Parse `nxm://game/mods/<mod>/files/<file>?key=…&expires=…`.
    pub fn parse(url: &str) -> Result<Self, HostError> {
        let rest = url
            .strip_prefix("nxm://")
            .ok_or_else(|| HostError::Other(format!("not an nxm:// url: {url}")))?;
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        let seg: Vec<&str> = path.split('/').collect();
        // [game, "mods", mod_id, "files", file_id]
        if seg.len() != 5 || seg[1] != "mods" || seg[3] != "files" {
            return Err(HostError::Other(format!(
                "malformed nxm url '{url}': expected nxm://<game>/mods/<id>/files/<id>"
            )));
        }
        let game = seg[0].to_string();
        if game.is_empty() {
            return Err(HostError::Other(format!("nxm url '{url}' has no game domain")));
        }
        let mod_id = seg[2]
            .parse()
            .map_err(|_| HostError::Other(format!("bad mod id '{}' in {url}", seg[2])))?;
        let file_id = seg[4]
            .parse()
            .map_err(|_| HostError::Other(format!("bad file id '{}' in {url}", seg[4])))?;

        let mut key = None;
        let mut expires = None;
        if let Some(q) = query {
            for pair in q.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    match k {
                        "key" => key = Some(v.to_string()),
                        "expires" => expires = Some(v.to_string()),
                        _ => {}
                    }
                }
            }
        }
        Ok(NxmUrl { game, mod_id, file_id, key, expires })
    }

    /// The Nexus `download_link.json` endpoint for this request, with the free-user
    /// `key`/`expires` query pair appended when present.
    pub fn api_url(&self) -> String {
        let mut url = format!(
            "https://api.nexusmods.com/v1/games/{}/mods/{}/files/{}/download_link.json",
            self.game, self.mod_id, self.file_id
        );
        if let (Some(k), Some(e)) = (&self.key, &self.expires) {
            url.push_str(&format!("?key={k}&expires={e}"));
        }
        url
    }
}

/// One CDN mirror from a `download_link.json` response.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DownloadLink {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(rename = "URI")]
    pub uri: String,
}

/// Parse the `download_link.json` array into CDN links.
pub fn parse_download_links(json: &str) -> Result<Vec<DownloadLink>, HostError> {
    serde_json::from_str(json)
        .map_err(|e| HostError::Other(format!("bad download_link.json response: {e}")))
}

/// Resolve an nxm url to a CDN link (via the Nexus API) and download it into
/// `dest_dir`, returning the saved file path. Requires the personal `apikey`.
/// Shells `curl`; the caller supplies the stored key.
pub fn download(url: &NxmUrl, api_key: &str, dest_dir: &Path) -> Result<PathBuf, HostError> {
    if which_curl().is_none() {
        return Err(HostError::Other(
            "curl not found on PATH; it's needed to fetch from Nexus".into(),
        ));
    }
    // 1. Ask the API for CDN links.
    let out = std::process::Command::new("curl")
        .arg("-sSf")
        .arg("-H")
        .arg(format!("apikey: {api_key}"))
        .arg("-H")
        .arg("Accept: application/json")
        .arg(url.api_url())
        .output()?;
    if !out.status.success() {
        return Err(HostError::Other(format!(
            "Nexus API request failed ({}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let links = parse_download_links(&String::from_utf8_lossy(&out.stdout))?;
    let link = links
        .first()
        .ok_or_else(|| HostError::Other("Nexus returned no download links".into()))?;

    // 2. Name the output after the file id (the CDN URL carries the real name in
    //    a query string we don't parse); the library renames on import anyway.
    std::fs::create_dir_all(dest_dir)?;
    let dest = dest_dir.join(format!("nexus_{}_{}.zip", url.mod_id, url.file_id));
    let dl = std::process::Command::new("curl")
        .arg("-sSfL")
        .arg("-o")
        .arg(&dest)
        .arg(&link.uri)
        .status()?;
    if !dl.success() {
        return Err(HostError::Other(format!(
            "download failed ({})",
            dl.code().unwrap_or(-1)
        )));
    }
    Ok(dest)
}

fn which_curl() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let c = dir.join(if cfg!(windows) { "curl.exe" } else { "curl" });
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

// --- OS scheme-handler registration -----------------------------------------

/// The freedesktop `.desktop` entry that makes aml the `nxm://` handler. `%u`
/// passes the clicked URL as the first arg to `aml nxm handle`.
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Aincrad Mod Loader (nxm handler)\n\
         Exec={} nxm handle %u\n\
         NoDisplay=true\n\
         MimeType=x-scheme-handler/nxm;\n",
        exe.display()
    )
}

/// Register aml as the system `nxm://` handler for the current user.
#[cfg(unix)]
pub fn register(exe: &Path) -> Result<PathBuf, HostError> {
    let apps = dirs::data_dir()
        .ok_or_else(|| HostError::Other("no data dir".into()))?
        .join("applications");
    std::fs::create_dir_all(&apps)?;
    let desktop = apps.join("aml-nxm.desktop");
    std::fs::write(&desktop, desktop_entry(exe))?;
    // Best-effort: point the nxm scheme at our entry and refresh the DB. These
    // tools may be absent on minimal systems; the .desktop file is the thing
    // that matters and it's already written.
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", "aml-nxm.desktop", "x-scheme-handler/nxm"])
        .status();
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps)
        .status();
    Ok(desktop)
}

/// Register aml as the `nxm://` handler via the Windows registry (HKCU).
#[cfg(windows)]
pub fn register(exe: &Path) -> Result<PathBuf, HostError> {
    let exe_s = exe.display().to_string();
    let root = r"HKCU\Software\Classes\nxm";
    let run = |args: &[&str]| -> Result<(), HostError> {
        let ok = std::process::Command::new("reg").args(args).status()?.success();
        if ok { Ok(()) } else { Err(HostError::Other(format!("reg {args:?} failed"))) }
    };
    run(&["add", root, "/ve", "/d", "URL:Aincrad Mod Loader", "/f"])?;
    run(&["add", root, "/v", "URL Protocol", "/d", "", "/f"])?;
    run(&[
        "add",
        &format!(r"{root}\shell\open\command"),
        "/ve",
        "/d",
        &format!("\"{exe_s}\" nxm handle \"%1\""),
        "/f",
    ])?;
    Ok(PathBuf::from(root))
}

/// Remove the `nxm://` handler registration.
#[cfg(unix)]
pub fn unregister() -> Result<(), HostError> {
    if let Some(data) = dirs::data_dir() {
        let desktop = data.join("applications/aml-nxm.desktop");
        if desktop.is_file() {
            std::fs::remove_file(desktop)?;
        }
    }
    Ok(())
}

/// Remove the Windows-registry `nxm://` handler registration.
#[cfg(windows)]
pub fn unregister() -> Result<(), HostError> {
    let _ = std::process::Command::new("reg")
        .args(["delete", r"HKCU\Software\Classes\nxm", "/f"])
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_url_with_tokens() {
        let u = NxmUrl::parse(
            "nxm://echoesofaincrad/mods/42/files/1337?key=abc123&expires=9999999999&user_id=7",
        )
        .unwrap();
        assert_eq!(u.game, "echoesofaincrad");
        assert_eq!(u.mod_id, 42);
        assert_eq!(u.file_id, 1337);
        assert_eq!(u.key.as_deref(), Some("abc123"));
        assert_eq!(u.expires.as_deref(), Some("9999999999"));
    }

    #[test]
    fn parse_premium_url_without_tokens() {
        let u = NxmUrl::parse("nxm://skyrim/mods/1/files/2").unwrap();
        assert_eq!(u.mod_id, 1);
        assert_eq!(u.file_id, 2);
        assert!(u.key.is_none());
    }

    #[test]
    fn parse_rejects_wrong_scheme_and_shape() {
        assert!(NxmUrl::parse("https://nexusmods.com/x").is_err());
        assert!(NxmUrl::parse("nxm://game/mods/42").is_err());
        assert!(NxmUrl::parse("nxm://game/plugins/42/files/1").is_err());
        assert!(NxmUrl::parse("nxm://game/mods/notanumber/files/1").is_err());
    }

    #[test]
    fn api_url_includes_tokens_when_present() {
        let u = NxmUrl::parse("nxm://g/mods/5/files/9?key=K&expires=E").unwrap();
        assert_eq!(
            u.api_url(),
            "https://api.nexusmods.com/v1/games/g/mods/5/files/9/download_link.json?key=K&expires=E"
        );
    }

    #[test]
    fn api_url_omits_query_for_premium() {
        let u = NxmUrl::parse("nxm://g/mods/5/files/9").unwrap();
        assert_eq!(
            u.api_url(),
            "https://api.nexusmods.com/v1/games/g/mods/5/files/9/download_link.json"
        );
    }

    #[test]
    fn parse_links_from_api_response() {
        let json = r#"[
            {"name":"Nexus CDN","short_name":"Nexus","URI":"https://cdn.nexusmods.com/file.zip?md5=x"},
            {"name":"Nexus CDN EU","short_name":"NexusEU","URI":"https://eu.cdn/file.zip"}
        ]"#;
        let links = parse_download_links(json).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].short_name, "Nexus");
        assert!(links[0].uri.starts_with("https://cdn.nexusmods.com/"));
    }

    #[test]
    fn desktop_entry_wires_scheme_and_exec() {
        let entry = desktop_entry(Path::new("/opt/aml/aml"));
        assert!(entry.contains("MimeType=x-scheme-handler/nxm;"));
        assert!(entry.contains("Exec=/opt/aml/aml nxm handle %u"));
    }
}
