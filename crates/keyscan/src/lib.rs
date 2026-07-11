//! aml-keyscan — recover the game's pak AES-256 key from the running process.
//!
//! We do **not** ship the key (that would be distributing the game's DRM material);
//! this recovers the user's own key from their own running game. UE holds the pak
//! key in memory at runtime; for Echoes of Aincrad it's a bare AES-NI key with no
//! resident 240-byte schedule, so we brute-validate every 16-byte-aligned 32-byte
//! window against a real pak's encrypted index (a match decrypts the mount-point
//! FString). Cross-platform: Linux `/proc/<pid>/mem`, Windows `ReadProcessMemory`.

use std::path::Path;

mod platform;

#[derive(Debug)]
pub enum KeyScanError {
    GameNotRunning,
    NoPak(String),
    NoKeyFound,
    Access(String),
}

impl std::fmt::Display for KeyScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            KeyScanError::GameNotRunning => {
                write!(f, "the game isn't running — launch Echoes of Aincrad (get into the world), then try again")
            }
            KeyScanError::NoPak(p) => write!(f, "couldn't read a pak to validate against: {p}"),
            KeyScanError::NoKeyFound => write!(
                f,
                "no key found — make sure the game is fully loaded (past the title), then try again{}",
                platform::PERMISSION_HINT
            ),
            KeyScanError::Access(e) => write!(f, "could not read the game's memory: {e}{}", platform::PERMISSION_HINT),
        }
    }
}
impl std::error::Error for KeyScanError {}

/// Find the running Echoes of Aincrad process id, if any.
pub fn find_game_pid() -> Option<u32> {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for (pid, proc_) in sys.processes() {
        let name = proc_.name().to_string_lossy().to_lowercase();
        let exe = proc_
            .exe()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.contains("echoesofaincrad") || exe.contains("echoesofaincrad") {
            return Some(pid.as_u32());
        }
    }
    None
}

/// Full path to the running game's executable, if found. Lets callers locate the
/// install (and thus its paks) from the live process — robust to non-default
/// Steam folders, other drives, and non-Steam installs, unlike scanning Steam's
/// default paths.
pub fn find_game_exe() -> Option<std::path::PathBuf> {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for proc_ in sys.processes().values() {
        let name = proc_.name().to_string_lossy().to_lowercase();
        let exe = proc_.exe();
        let exe_name = exe
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.contains("echoesofaincrad") || exe_name.contains("echoesofaincrad") {
            if let Some(e) = exe {
                return Some(e.to_path_buf());
            }
        }
    }
    None
}

/// Recover the pak AES key from the running game, validating against `pak_path`.
/// Returns the key as a `0x…` hex string on success.
pub fn recover_key(pak_path: &Path) -> Result<String, KeyScanError> {
    let enc = pak_index_block(pak_path)
        .ok_or_else(|| KeyScanError::NoPak(pak_path.display().to_string()))?;
    let pid = find_game_pid().ok_or(KeyScanError::GameNotRunning)?;
    scan_process(pid, &enc)
}

/// Scan process `pid`'s readable memory for a key that decrypts `enc`.
pub fn scan_process(pid: u32, enc: &[u8; 16]) -> Result<String, KeyScanError> {
    let mem = platform::ProcMem::open(pid).map_err(KeyScanError::Access)?;
    const CHUNK: usize = 4 * 1024 * 1024;
    const OVERLAP: usize = 32;
    for (start, end) in mem.regions() {
        if end <= start || end - start > 4 * 1024 * 1024 * 1024 {
            continue;
        }
        let mut off = start;
        while off < end {
            let want = ((end - off) as usize).min(CHUNK + OVERLAP);
            let mut buf = vec![0u8; want];
            if let Some(n) = mem.read_at(off, &mut buf) {
                if let Some(key) = scan_buffer(&buf[..n], enc) {
                    return Ok(hex_key(&key));
                }
            }
            off += CHUNK as u64;
        }
    }
    Err(KeyScanError::NoKeyFound)
}

/// Brute-validate every 16-byte-aligned 32-byte window in `buf` against `enc`.
fn scan_buffer(buf: &[u8], enc: &[u8; 16]) -> Option<[u8; 32]> {
    let mut i = 0;
    while i + 32 <= buf.len() {
        let key: [u8; 32] = buf[i..i + 32].try_into().unwrap();
        if key_decrypts_pak(&key, enc) {
            return Some(key);
        }
        i += 16;
    }
    None
}

fn hex_key(bytes: &[u8]) -> String {
    let mut s = String::from("0x");
    for b in bytes {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

/// Parse a UE pak footer: find magic 0x5A6F12E1 near EOF, return the encrypted
/// index's first 16-byte block (what we brute-decrypt to validate a key).
pub fn pak_index_block(pak_path: &Path) -> Option<[u8; 16]> {
    let data = std::fs::read(pak_path).ok()?;
    let magic = [0xE1u8, 0x12, 0x6F, 0x5A];
    let start = data.len().saturating_sub(1024);
    let mut foot = None;
    for i in (start..data.len().saturating_sub(4)).rev() {
        if data[i..i + 4] == magic {
            foot = Some(i);
            break;
        }
    }
    let m = foot?;
    // Layout after magic(4)+version(4): IndexOffset(8) IndexSize(8) …
    let off = m + 8;
    let index_offset = u64::from_le_bytes(data.get(off..off + 8)?.try_into().ok()?) as usize;
    let mut block = [0u8; 16];
    block.copy_from_slice(data.get(index_offset..index_offset + 16)?);
    Some(block)
}

/// Decrypt the pak's index block with `key` (AES-256 ECB) and check the plaintext
/// looks like a pak index (an FString mount point: int32 length then '.'/'/').
pub fn key_decrypts_pak(key: &[u8; 32], enc: &[u8; 16]) -> bool {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
    let cipher = aes::Aes256::new(GenericArray::from_slice(key));
    let mut b = *GenericArray::from_slice(enc);
    cipher.decrypt_block(&mut b);
    let len = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    (2..512).contains(&len) && (b[4] == b'.' || b[4] == b'/')
}
