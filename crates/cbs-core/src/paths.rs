//! Per-OS locations - the same folders the Python version uses, so both read the same
//! settings and models.
//!
//! | | Linux | Windows |
//! |---|---|---|
//! | settings | `~/.config/chatbox-studio` | `%APPDATA%\Chatbox Studio` |
//! | data (models, logs) | `~/.local/share/chatbox-studio` | `%LOCALAPPDATA%\Chatbox Studio` |

use std::path::{Path, PathBuf};

pub const IS_WINDOWS: bool = cfg!(windows);
pub const WIN_NAME: &str = "Chatbox Studio";
pub const LINUX_NAME: &str = "chatbox-studio";
/// The name up to version 1.8 ("VRC Voice Translator").
pub const OLD_WIN_NAME: &str = "VRC Voice Translator";
pub const OLD_LINUX_NAME: &str = "vrc-voice-translator";

/// An environment variable as a path, `None` when unset *or empty* (like Python's `or`).
fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home() -> PathBuf {
    env_path(if IS_WINDOWS { "USERPROFILE" } else { "HOME" })
        .or_else(|| env_path("HOME"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn win_roaming() -> PathBuf {
    env_path("APPDATA").unwrap_or_else(|| home().join("AppData").join("Roaming"))
}

fn win_local() -> PathBuf {
    env_path("LOCALAPPDATA").unwrap_or_else(|| home().join("AppData").join("Local"))
}

fn xdg_config() -> PathBuf {
    env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
}

fn xdg_data() -> PathBuf {
    env_path("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share"))
}

/// Settings folder. `old`: the folder of the old name.
pub fn config_dir_of(old: bool) -> PathBuf {
    if IS_WINDOWS {
        win_roaming().join(if old { OLD_WIN_NAME } else { WIN_NAME })
    } else {
        xdg_config().join(if old { OLD_LINUX_NAME } else { LINUX_NAME })
    }
}

/// App data: models, engines, logs.
pub fn data_dir_of(old: bool) -> PathBuf {
    if IS_WINDOWS {
        win_local().join(if old { OLD_WIN_NAME } else { WIN_NAME })
    } else {
        xdg_data().join(if old { OLD_LINUX_NAME } else { LINUX_NAME })
    }
}

pub fn config_dir() -> PathBuf {
    config_dir_of(false)
}

pub fn data_dir() -> PathBuf {
    data_dir_of(false)
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn crash_log() -> PathBuf {
    data_dir().join("crash.log")
}

pub fn starts_log() -> PathBuf {
    data_dir().join("starts.log")
}

/// Hugging Face cache (Whisper models of the CTranslate2 engine).
pub fn hf_cache() -> PathBuf {
    env_path("HF_HUB_CACHE")
        .or_else(|| env_path("HF_HOME").map(|p| p.join("hub")))
        .or_else(|| env_path("XDG_CACHE_HOME").map(|p| p.join("huggingface").join("hub")))
        .unwrap_or_else(|| home().join(".cache").join("huggingface").join("hub"))
}

/// Carries settings and models over from "VRC Voice Translator" (1.0-1.8). Returns what
/// was taken over, e.g. `["settings", "translator model"]`.
pub fn migrate_from_old_name() -> Vec<&'static str> {
    let mut moved = Vec::new();
    let old_cfg = config_dir_of(true).join("settings.json");
    let new_cfg = settings_file();
    if old_cfg.exists() && !new_cfg.exists() {
        if let Some(parent) = new_cfg.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::copy(&old_cfg, &new_cfg).is_ok() {
            moved.push("settings");
        }
    }
    let old_models = data_dir_of(true).join("models");
    let new_models = models_dir();
    if old_models.is_dir() && !dir_has_entries(&new_models) {
        if let Some(parent) = new_models.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if new_models.exists() {
            let _ = std::fs::remove_dir(&new_models);
        }
        if std::fs::rename(&old_models, &new_models).is_ok() {
            moved.push("translator model");
        }
    }
    moved
}

fn dir_has_entries(p: &Path) -> bool {
    std::fs::read_dir(p).map(|mut d| d.next().is_some()).unwrap_or(false)
}
