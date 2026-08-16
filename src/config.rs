use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::provider::presets;

/// Environment variable that overrides the config's key for any provider.
const KEY_ENV: &str = "PLZ_API_KEY";

/// Environment variable that points at a different config file.
/// Used by tests and for keeping several profiles.
const CONFIG_PATH_ENV: &str = "PLZ_CONFIG";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Preset name from `provider::presets`, or "custom"
    #[serde(default)]
    pub preset: String,
    /// Base URL without a trailing slash
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    /// Stored here only when it is not supplied through an environment variable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            preset: presets::DEEPSEEK.name.to_string(),
            base_url: presets::DEEPSEEK.base_url.to_string(),
            model: presets::DEEPSEEK.model.to_string(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    /// How many suggestions to request
    #[serde(default = "default_suggestions")]
    pub suggestions: usize,
    /// Ask for confirmation before running risky commands
    #[serde(default = "default_true")]
    pub confirm_dangerous: bool,
    /// Timeout for the HTTP request to the model
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Whether to send the working directory to the model
    #[serde(default = "default_true")]
    pub send_cwd: bool,
    /// Whether to ask the endpoint for `response_format: json_object`
    #[serde(default = "default_true")]
    pub json_mode: bool,
    /// Whether to ask the endpoint to skip reasoning with
    /// `thinking: {"type": "disabled"}`
    ///
    /// Off unless a preset asks for it: the field is not part of the OpenAI API,
    /// and an endpoint that does not know it answers 400.
    #[serde(default)]
    pub disable_thinking: bool,
    /// Interface language, or "auto" to follow the OS
    ///
    /// A free string rather than an enum: an unknown value has to degrade to
    /// auto-detection, because a config that fails to parse takes plz down with
    /// it. Left out of files that do not set it, so nothing changes for anyone
    /// who does not care.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

fn default_suggestions() -> usize {
    3
}
fn default_true() -> bool {
    true
}
fn default_timeout() -> u64 {
    30
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            suggestions: default_suggestions(),
            confirm_dangerous: default_true(),
            timeout_secs: default_timeout(),
            send_cwd: default_true(),
            json_mode: default_true(),
            // Follows the default provider above, which is DeepSeek.
            disable_thinking: presets::DEEPSEEK.disable_thinking,
            language: None,
        }
    }
}

impl Config {
    /// Path to the config file for the current OS.
    ///
    /// Linux:   ~/.config/plz/config.toml
    /// macOS:   ~/Library/Application Support/plz/config.toml
    /// Windows: %APPDATA%\plz\config.toml
    pub fn path() -> Result<PathBuf> {
        if let Some(custom) = std::env::var_os(CONFIG_PATH_ENV) {
            return Ok(PathBuf::from(custom));
        }
        let dirs = directories::ProjectDirs::from("", "", "plz")
            .ok_or_else(|| anyhow!("{}", t!("errors.no_config_dir")))?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load the config from disk. A missing file is an error with a hint
    /// rather than silent defaults: without a key plz cannot work anyway.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Err(anyhow!("{}", t!("errors.no_config", path = path.display())));
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| t!("errors.could_not_read", path = path.display()).to_string())?;
        let config: Config = toml::from_str(&raw)
            .with_context(|| t!("errors.could_not_parse", path = path.display()).to_string())?;
        Ok(config)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let raw = toml::to_string_pretty(self)
            .with_context(|| t!("errors.could_not_serialize").to_string())?;
        write_private(&path, &raw)?;
        Ok(path)
    }

    /// The API key, resolved in priority order:
    /// `PLZ_API_KEY`, then the preset's variable (`DEEPSEEK_API_KEY` and the
    /// like), then the config field.
    ///
    /// Environment variables come first so that CI and temporary switches do
    /// not require editing the file.
    pub fn api_key(&self) -> Option<String> {
        if let Ok(key) = std::env::var(KEY_ENV) {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
        if let Some(preset) = presets::by_name(&self.provider.preset) {
            if let Some(var) = preset.key_env {
                if let Ok(key) = std::env::var(var) {
                    if !key.trim().is_empty() {
                        return Some(key);
                    }
                }
            }
        }
        self.provider
            .api_key
            .as_ref()
            .filter(|k| !k.trim().is_empty())
            .cloned()
    }

    /// Whether the chosen endpoint needs a key (a local Ollama does not).
    pub fn key_required(&self) -> bool {
        presets::by_name(&self.provider.preset)
            .map(|p| p.key_env.is_some())
            .unwrap_or(true)
    }

    /// A copy of the config that is safe to print.
    pub fn redacted(&self) -> Config {
        let mut copy = self.clone();
        copy.provider.api_key = copy.provider.api_key.map(|k| mask_key(&k));
        copy
    }
}

/// Mask a key, keeping both ends so it stays recognisable.
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}...{tail}")
}

/// Write a file only its owner can read (0600).
///
/// The mode is part of the create call rather than a `chmod` afterwards: with
/// a plain write the key lands on disk under whatever the umask allows —
/// usually world-readable — and stays that way until the next syscall. The
/// window is short, but the file is a long-lived secret.
///
/// Windows has no equivalent of Unix modes; there the file already lives in the
/// user's profile, which other unprivileged accounts cannot reach.
#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| t!("errors.could_not_write", path = path.display()).to_string())?;

    // `mode` applies only to a file this call creates, so a config left behind
    // by an older version keeps its old permissions — tighten those too, while
    // the file is still empty.
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| t!("errors.could_not_set_mode", path = path.display()).to_string())?;

    file.write_all(contents.as_bytes())
        .with_context(|| t!("errors.could_not_write", path = path.display()).to_string())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)
        .with_context(|| t!("errors.could_not_write", path = path.display()).to_string())
}

/// Create the config directory, on Unix reachable only by its owner (0700).
#[cfg(unix)]
fn create_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    // The mode applies to the directories this call creates; ones that already
    // exist — `~/.config`, `~/Library/Application Support` — are left alone,
    // which is not ours to tighten.
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .with_context(|| t!("errors.could_not_create_dir", path = path.display()).to_string())
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| t!("errors.could_not_create_dir", path = path.display()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env_guard;

    fn clear_key_vars() {
        std::env::remove_var(KEY_ENV);
        for preset in presets::ALL {
            if let Some(var) = preset.key_env {
                std::env::remove_var(var);
            }
        }
    }

    #[test]
    fn config_round_trips_through_toml() {
        let config = Config::default();
        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.provider.preset, config.provider.preset);
        assert_eq!(parsed.provider.base_url, config.provider.base_url);
        assert_eq!(parsed.behavior.suggestions, config.behavior.suggestions);
        assert!(parsed.behavior.confirm_dangerous);
    }

    #[test]
    fn an_unset_language_stays_out_of_the_written_file() {
        // The key is opt-in: writing `language = ""` back into everyone's
        // config would be a visible change for people who never asked for one.
        let config = Config::default();
        assert!(config.behavior.language.is_none());
        assert!(!toml::to_string_pretty(&config)
            .unwrap()
            .contains("language"));

        let parsed: Config = toml::from_str("[behavior]\nlanguage = \"fr\"\n").unwrap();
        assert_eq!(parsed.behavior.language.as_deref(), Some("fr"));
    }

    #[test]
    fn empty_toml_falls_back_to_defaults() {
        let parsed: Config = toml::from_str("").unwrap();
        assert_eq!(parsed.behavior.suggestions, 3);
        assert!(parsed.behavior.confirm_dangerous);
        assert_eq!(parsed.provider.preset, presets::DEEPSEEK.name);
    }

    #[test]
    fn plz_api_key_wins_over_config_field() {
        let _guard = env_guard();
        clear_key_vars();
        let mut config = Config::default();
        config.provider.api_key = Some("from-config".into());
        std::env::set_var(KEY_ENV, "from-env");

        assert_eq!(config.api_key().as_deref(), Some("from-env"));

        clear_key_vars();
        assert_eq!(config.api_key().as_deref(), Some("from-config"));
    }

    #[test]
    fn preset_env_var_wins_over_config_field() {
        let _guard = env_guard();
        clear_key_vars();
        let mut config = Config::default(); // deepseek preset
        config.provider.api_key = Some("from-config".into());
        std::env::set_var("DEEPSEEK_API_KEY", "from-preset-env");

        assert_eq!(config.api_key().as_deref(), Some("from-preset-env"));
        clear_key_vars();
    }

    #[test]
    fn blank_env_var_does_not_shadow_config_field() {
        let _guard = env_guard();
        clear_key_vars();
        let mut config = Config::default();
        config.provider.api_key = Some("from-config".into());
        std::env::set_var(KEY_ENV, "   ");

        assert_eq!(config.api_key().as_deref(), Some("from-config"));
        clear_key_vars();
    }

    #[test]
    fn ollama_does_not_require_a_key() {
        let mut config = Config::default();
        config.provider.preset = presets::OLLAMA.name.to_string();
        assert!(!config.key_required());
    }

    #[test]
    fn saved_file_is_owner_only_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let _guard = env_guard();
        std::env::set_var(CONFIG_PATH_ENV, &path);

        let mut config = Config::default();
        config.provider.api_key = Some("secret".into());
        config.save().unwrap();

        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "the key must not be readable by others"
            );
        }

        std::env::remove_var(CONFIG_PATH_ENV);
    }

    #[test]
    #[cfg(unix)]
    fn an_existing_loose_config_is_tightened_on_save() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let _guard = env_guard();
        std::env::set_var(CONFIG_PATH_ENV, &path);

        // Left behind by an older version, or by a hand-edit under a loose umask.
        fs::write(&path, "# stale\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let mut config = Config::default();
        config.provider.api_key = Some("secret".into());
        config.save().unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "an old loose file must be tightened");
        assert!(fs::read_to_string(&path).unwrap().contains("secret"));

        std::env::remove_var(CONFIG_PATH_ENV);
    }

    #[test]
    #[cfg(unix)]
    fn the_config_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("plz");
        let path = nested.join("config.toml");
        let _guard = env_guard();
        std::env::set_var(CONFIG_PATH_ENV, &path);

        Config::default().save().unwrap();

        let mode = fs::metadata(&nested).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);

        std::env::remove_var(CONFIG_PATH_ENV);
    }

    #[test]
    fn redacted_hides_the_middle_of_the_key() {
        let mut config = Config::default();
        config.provider.api_key = Some("sk-1234567890abcdef".into());
        let shown = config.redacted().provider.api_key.unwrap();
        assert!(!shown.contains("567890"));
        assert!(shown.starts_with("sk-1"));
        assert!(shown.ends_with("cdef"));
    }

    #[test]
    fn short_keys_are_fully_masked() {
        assert_eq!(mask_key("short"), "*****");
    }
}
