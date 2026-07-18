//! Voice preview — plays the character creator's own per-voice sample lines so
//! a voice can be auditioned without booting the game.
//!
//! Clips ship in the bundle payload (not version-controlled, like thumbnails)
//! as `<data>/voices/<lang>/<Voice>_<n>.ogg`, n = 1..=6 — the exact lines the
//! in-game creator plays, extracted from the `Play_VOFX_AvatarCustomize` Wwise
//! event (see `scripts/extract-voices.py`). A missing payload hides the
//! preview UI entirely; a missing/failed audio device degrades to silence.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

/// The game ships creator voice lines in two audio languages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AudioLang {
    En,
    Jp,
}

impl AudioLang {
    pub const ALL: [AudioLang; 2] = [AudioLang::En, AudioLang::Jp];

    pub fn dir(self) -> &'static str {
        match self {
            AudioLang::En => "en",
            AudioLang::Jp => "jp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioLang::En => "EN",
            AudioLang::Jp => "JP",
        }
    }
}

pub struct VoicePreview {
    dir: PathBuf,
    pub lang: AudioLang,
    /// Per language: voice name -> sample-line numbers found on disk (sorted).
    /// Scanned once at startup; the payload never changes mid-session.
    lines: HashMap<&'static str, HashMap<String, Vec<u32>>>,
    /// Round-robin cursor per voice so repeat presses cycle through the lines.
    next: HashMap<String, usize>,
    /// The output device, opened lazily on the first play (it must stay alive
    /// for playback to continue). `Err`-once means we stop retrying.
    device: Option<rodio::MixerDeviceSink>,
    device_failed: bool,
    /// Current playback; replaced (stopped) on each new play so presses never
    /// overlap.
    player: Option<rodio::Player>,
}

impl VoicePreview {
    pub fn new(dir: PathBuf, lang: AudioLang) -> Self {
        let mut lines = HashMap::new();
        for l in AudioLang::ALL {
            lines.insert(l.dir(), scan(&dir, l));
        }
        Self {
            dir,
            lang,
            lines,
            next: HashMap::new(),
            device: None,
            device_failed: false,
            player: None,
        }
    }

    /// Any clips at all (current language)? Gates the whole preview UI.
    pub fn any(&self) -> bool {
        self.lines.get(self.lang.dir()).is_some_and(|m| !m.is_empty())
    }

    pub fn lang_available(&self, lang: AudioLang) -> bool {
        self.lines.get(lang.dir()).is_some_and(|m| !m.is_empty())
    }

    /// Play the next sample line for this voice (cycling through the creator's
    /// lines on repeat presses). Silently no-ops on any failure — preview is
    /// best-effort garnish, never worth an error dialog.
    pub fn play(&mut self, voice: &str) {
        let Some(lines) = self.lines.get(self.lang.dir()).and_then(|m| m.get(voice)) else {
            return;
        };
        if lines.is_empty() {
            return;
        }
        let cursor = self.next.entry(voice.to_string()).or_insert(0);
        let n = lines[*cursor % lines.len()];
        *cursor = (*cursor + 1) % lines.len();

        if self.device.is_none() && !self.device_failed {
            match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(d) => self.device = Some(d),
                Err(_) => self.device_failed = true,
            }
        }
        let Some(device) = &self.device else { return };
        let path = self.dir.join(self.lang.dir()).join(format!("{voice}_{n}.ogg"));
        let Ok(file) = File::open(&path) else { return };
        let Ok(decoded) = rodio::Decoder::try_from(file) else { return };
        if let Some(old) = self.player.take() {
            old.stop();
        }
        let player = rodio::Player::connect_new(device.mixer());
        player.append(decoded);
        self.player = Some(player);
    }
}

/// Scan `<dir>/<lang>/` once for `<Voice>_<n>.ogg` clips.
fn scan(dir: &Path, lang: AudioLang) -> HashMap<String, Vec<u32>> {
    let mut m: HashMap<String, Vec<u32>> = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir.join(lang.dir())) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "ogg") {
                continue;
            }
            if let Some((voice, n)) = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.rsplit_once('_'))
            {
                if let Ok(n) = n.parse() {
                    m.entry(voice.to_string()).or_default().push(n);
                }
            }
        }
    }
    for v in m.values_mut() {
        v.sort_unstable();
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<Voice>_<n>.ogg` stems must split back into exactly the voice names the
    /// picker uses — the trailing `_<n>` split must not eat `Player_M_02`'s
    /// numeric suffix.
    #[test]
    fn stem_split_preserves_voice_names() {
        for (stem, want) in [
            ("Player_M_1", "Player_M"),
            ("Player_M_02_6", "Player_M_02"),
            ("Player_F_1", "Player_F"),
            ("Player_F_06_3", "Player_F_06"),
        ] {
            let (voice, n) = stem.rsplit_once('_').unwrap();
            assert_eq!(voice, want);
            assert!(n.parse::<u32>().is_ok());
        }
    }

    /// Every voice the picker offers resolves to clips in BOTH languages when
    /// the payload is staged (dev machines / packaged bundles). Gated: absent
    /// payload skips, matching the local-save-gated tests in aml-save.
    #[test]
    fn every_picker_voice_has_clips_in_both_langs() {
        let dir = crate::locate::voices_dir();
        if !dir.is_dir() {
            eprintln!("skipping: no voices payload at {}", dir.display());
            return;
        }
        let pv = VoicePreview::new(dir, AudioLang::En);
        for lang in AudioLang::ALL {
            assert!(pv.lang_available(lang), "no clips for {}", lang.dir());
            let m = pv.lines.get(lang.dir()).unwrap();
            for v in aml_save::appearance::MALE_VOICES
                .iter()
                .chain(aml_save::appearance::FEMALE_VOICES.iter())
            {
                let lines = m.get(*v).unwrap_or_else(|| panic!("{}: no clips for {v}", lang.dir()));
                assert!(!lines.is_empty(), "{}: empty clip list for {v}", lang.dir());
            }
        }
    }

    /// The staged clips must actually decode with the decoder the app ships
    /// (symphonia ogg/vorbis) — catches a payload encoded in a format the
    /// binary can't play. Gated on the payload like above.
    #[test]
    fn staged_clips_decode() {
        let dir = crate::locate::voices_dir();
        if !dir.is_dir() {
            eprintln!("skipping: no voices payload at {}", dir.display());
            return;
        }
        let mut checked = 0;
        for lang in AudioLang::ALL {
            let sub = dir.join(lang.dir());
            let Ok(rd) = std::fs::read_dir(&sub) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "ogg") {
                    let f = File::open(&p).unwrap();
                    rodio::Decoder::try_from(f)
                        .unwrap_or_else(|e| panic!("{} does not decode: {e}", p.display()));
                    checked += 1;
                }
            }
        }
        assert!(checked == 0 || checked >= 144, "partial payload: {checked} clips");
    }
}
