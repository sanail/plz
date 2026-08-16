use rust_i18n::t;

/// A ready-made setup for one OpenAI-compatible endpoint.
///
/// Everything here is only a default for the config. Users can override any of
/// it by hand, and `Preset::CUSTOM` lets them point at an arbitrary `base_url`
/// with no preset at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// The name written to the config's `preset` field
    pub name: &'static str,
    /// Human-readable title shown during interactive setup
    pub title: &'static str,
    /// Base URL without a trailing slash; `/chat/completions` is appended to it
    pub base_url: &'static str,
    /// Default model
    pub model: &'static str,
    /// Environment variable holding the key when the config has none
    pub key_env: Option<&'static str>,
    /// Whether the endpoint supports `response_format: {"type": "json_object"}`
    pub json_mode: bool,
    /// Whether to ask the endpoint to skip reasoning with
    /// `thinking: {"type": "disabled"}`. Set it only where the default model
    /// thinks: here reasoning buys nothing and costs latency and tokens.
    pub disable_thinking: bool,
    /// Where to get a key; shown during first-time setup
    pub key_hint: &'static str,
}

pub const DEEPSEEK: Preset = Preset {
    name: "deepseek",
    title: "DeepSeek",
    base_url: "https://api.deepseek.com/v1",
    model: "deepseek-v4-flash",
    key_env: Some("DEEPSEEK_API_KEY"),
    json_mode: true,
    // The model thinks by default, and a one-shot CLI gains nothing from it.
    disable_thinking: true,
    key_hint: "https://platform.deepseek.com/api_keys",
};

pub const OPENAI: Preset = Preset {
    name: "openai",
    title: "OpenAI",
    base_url: "https://api.openai.com/v1",
    model: "gpt-5.6-luna",
    key_env: Some("OPENAI_API_KEY"),
    json_mode: true,
    disable_thinking: false,
    key_hint: "https://platform.openai.com/api-keys",
};

pub const OPENROUTER: Preset = Preset {
    name: "openrouter",
    title: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    // The `~` prefix is OpenRouter's router alias: it resolves to the newest
    // model of the family, so this preset does not go stale — and eventually
    // 404 — each time a dated Flash release replaces the previous one. The bare
    // `deepseek/deepseek-v4-flash` is a browsable page, not an id the API
    // accepts: the catalogue at /api/v1/models carries only the dated slugs and
    // this alias.
    model: "~deepseek/deepseek-v4-flash-latest",
    key_env: Some("OPENROUTER_API_KEY"),
    json_mode: true,
    // A proxy in front of many providers: an unknown field in the body is
    // likelier to be rejected than honoured.
    disable_thinking: false,
    key_hint: "https://openrouter.ai/keys",
};

pub const OLLAMA: Preset = Preset {
    name: "ollama",
    title: "Ollama (local)",
    base_url: "http://localhost:11434/v1",
    model: "qwen3.5",
    key_env: None,
    json_mode: false,
    disable_thinking: false,
    key_hint: "no key required",
};

pub const CUSTOM: Preset = Preset {
    name: "custom",
    title: "Another OpenAI-compatible endpoint",
    base_url: "",
    model: "",
    key_env: None,
    json_mode: true,
    disable_thinking: false,
    key_hint: "see your provider's documentation",
};

/// Every preset, in the order they appear during first-time setup.
pub const ALL: &[Preset] = &[DEEPSEEK, OPENAI, OPENROUTER, OLLAMA, CUSTOM];

impl Preset {
    /// The base URL as shown in the selection list.
    ///
    /// `custom` has none — the user types the address themselves, and empty
    /// parentheses in the list would look like a bug.
    pub fn base_url_display(&self) -> String {
        if self.base_url.is_empty() {
            t!("wizard.address_entered_manually").to_string()
        } else {
            self.base_url.to_string()
        }
    }

    /// The title as shown in the setup list.
    ///
    /// The struct keeps the English text as the preset's identity; only the two
    /// entries that are prose rather than a brand name have a translation.
    pub fn title_display(&self) -> String {
        match self.name {
            n if n == OLLAMA.name => t!("wizard.preset_ollama").to_string(),
            n if n == CUSTOM.name => t!("wizard.preset_custom").to_string(),
            _ => self.title.to_string(),
        }
    }

    /// Where to get a key. For most presets this is a URL, which stays as it is.
    pub fn key_hint_display(&self) -> String {
        match self.name {
            n if n == OLLAMA.name => t!("wizard.no_key_required").to_string(),
            n if n == CUSTOM.name => t!("wizard.see_provider_docs").to_string(),
            _ => self.key_hint.to_string(),
        }
    }
}

/// Look up a preset by the name stored in the config.
pub fn by_name(name: &str) -> Option<Preset> {
    ALL.iter().find(|p| p.name == name).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_are_findable_by_name() {
        for preset in ALL {
            assert_eq!(by_name(preset.name).map(|p| p.name), Some(preset.name));
        }
    }

    #[test]
    fn unknown_preset_is_none() {
        assert!(by_name("no-such-provider").is_none());
    }

    #[test]
    fn only_deepseek_disables_thinking() {
        // The field puts an extra key in the request body, and endpoints that do
        // not know it answer 400. Turning it on is a per-provider decision.
        for preset in ALL {
            assert_eq!(
                preset.disable_thinking,
                preset.name == DEEPSEEK.name,
                "{}",
                preset.name
            );
        }
    }

    #[test]
    fn base_urls_have_no_trailing_slash() {
        // "/chat/completions" is appended to base_url; a trailing slash would give "//".
        for preset in ALL {
            assert!(!preset.base_url.ends_with('/'), "{}", preset.name);
        }
    }
}
