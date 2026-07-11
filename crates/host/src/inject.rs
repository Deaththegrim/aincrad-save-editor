//! UE4SS injection strategy, abstracted over Windows-native vs Proton.
//!
//! UE4SS injects via a proxy DLL (`dwmapi.dll`) placed next to the game exe.
//! - On **Windows** the OS loads that DLL automatically; nothing else needed.
//! - Under **Proton/Wine** the built-in `dwmapi` must be overridden so our proxy
//!   wins the load. That's done with `WINEDLLOVERRIDES=dwmapi=n,b`, which we pass
//!   through Steam's launch options.

use crate::detect::Runtime;

/// The concrete steps required to make UE4SS load for a given runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectionStrategy {
    /// Env vars that must be present when the game launches.
    pub env: Vec<(String, String)>,
    /// A human-readable Steam "launch options" string, if the user launches via
    /// Steam rather than through us.
    pub steam_launch_options: Option<String>,
    /// Notes to surface to the user (`aml doctor`).
    pub notes: Vec<String>,
}

impl InjectionStrategy {
    pub fn for_runtime(runtime: &Runtime) -> Self {
        match runtime {
            Runtime::WindowsNative => InjectionStrategy {
                env: vec![],
                steam_launch_options: None,
                notes: vec![
                    "Windows: the proxy DLL (dwmapi.dll) in Binaries/Win64 loads \
                     automatically. No launch options required."
                        .to_string(),
                ],
            },
            Runtime::Proton { .. } => {
                // n,b = native then builtin: our proxy DLL first, real one as fallback.
                let overrides = "dwmapi=n,b".to_string();
                InjectionStrategy {
                    env: vec![("WINEDLLOVERRIDES".to_string(), overrides.clone())],
                    steam_launch_options: Some(format!(
                        "WINEDLLOVERRIDES=\"{overrides}\" %command%"
                    )),
                    notes: vec![
                        "Proton: the built-in dwmapi must be overridden so UE4SS's \
                         proxy DLL loads. Set the Steam launch option below, or launch \
                         through `aml launch` which sets it for you."
                            .to_string(),
                        "If UE4SS still doesn't load, confirm the game's Proton version \
                         and that Steam Play is enabled for this title."
                            .to_string(),
                    ],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn windows_needs_no_env() {
        let s = InjectionStrategy::for_runtime(&Runtime::WindowsNative);
        assert!(s.env.is_empty());
        assert!(s.steam_launch_options.is_none());
    }

    #[test]
    fn proton_sets_dll_override() {
        let s = InjectionStrategy::for_runtime(&Runtime::Proton {
            prefix: PathBuf::from("/tmp/pfx"),
        });
        assert_eq!(
            s.env,
            vec![("WINEDLLOVERRIDES".to_string(), "dwmapi=n,b".to_string())]
        );
        assert!(s.steam_launch_options.unwrap().contains("%command%"));
    }
}
