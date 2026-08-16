//! The interface language: which one to use, and how it is detected.
//!
//! The messages themselves live in `locales/*.toml` and are reached through
//! `t!`; this module only decides which column of those files is read.

/// A language plz ships translations for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
    Es,
    Fr,
    De,
}

impl Lang {
    /// Every language, for tests that have to cover all of them.
    #[cfg(test)]
    pub const ALL: [Lang; 5] = [Lang::En, Lang::Ru, Lang::Es, Lang::Fr, Lang::De];

    /// The locale code, which is also the key of the column in the catalogues.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }

    /// The English name of the language, for the system prompt.
    ///
    /// An English name inside an English prompt is what models follow most
    /// reliably — more so than a locale code or the endonym.
    pub fn english_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Russian",
            Lang::Es => "Spanish",
            Lang::Fr => "French",
            Lang::De => "German",
        }
    }

    /// Match a locale tag against the supported languages.
    ///
    /// Accepts what the platforms actually hand out: `ru_RU.UTF-8`, `fr-CA`,
    /// `es_ES@euro`, a bare `de`. Only the primary subtag decides, so `fr-CA`
    /// and `fr-FR` are one language here — regional catalogues would be four
    /// more files for no gain. `C`, `POSIX` and every unsupported language give
    /// `None`, which the caller reads as English.
    pub fn from_tag(raw: &str) -> Option<Self> {
        let tag = raw.trim().split(['.', '@']).next()?;
        match tag.split(['_', '-']).next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "ru" => Some(Lang::Ru),
            "es" => Some(Lang::Es),
            "fr" => Some(Lang::Fr),
            "de" => Some(Lang::De),
            _ => None,
        }
    }
}

/// The first supported language in a list, for `LANGUAGE=it:de_DE:en`.
fn first_supported(list: &str) -> Option<Lang> {
    list.split([':', ',']).find_map(Lang::from_tag)
}

/// Pick the language from the sources, in priority order.
///
/// `forced` comes first so a single run can be overridden without touching
/// anything on disk, then the config, then what the OS reports.
///
/// An explicit request settles the matter even when plz ships no catalogue for
/// it: someone who asks for Italian gets English, not whatever language the
/// machine happens to be set to. Only the ways of asking for nothing — unset,
/// blank, or the default `auto` — hand the decision down.
fn resolve(forced: Option<&str>, configured: Option<&str>, detected: Option<&str>) -> Lang {
    for request in [forced, configured] {
        match request.map(str::trim) {
            None | Some("") | Some("auto") => continue,
            Some(tag) => return first_supported(tag).unwrap_or(Lang::En),
        }
    }
    detected.and_then(first_supported).unwrap_or(Lang::En)
}

/// Resolve the interface language and apply it to the whole process.
///
/// `PLZ_LANG` overrides the config, in the same shape as `PLZ_API_KEY` and
/// `PLZ_CONFIG`. The OS answer comes from the platform rather than from `LANG`:
/// macOS reports the region there, and Windows sets it at all only under MSYS2.
pub fn init(configured: Option<&str>) {
    let forced = std::env::var("PLZ_LANG").ok();
    let detected = sys_locale::get_locale();
    let lang = resolve(forced.as_deref(), configured, detected.as_deref());
    rust_i18n::set_locale(lang.code());
}

/// The language currently in force, for the few decisions that are not just a
/// lookup — which spelling of "yes" to accept, which language to ask the model
/// to answer in.
pub fn current() -> Lang {
    Lang::from_tag(&rust_i18n::locale()).unwrap_or(Lang::En)
}

/// Checks over the catalogue files themselves.
///
/// `t!` resolves its keys at runtime, so nothing here is caught by the
/// compiler: a language missing from an entry, a dropped placeholder or a
/// translated config key all build cleanly and surface in front of a user.
/// These read the same files the macro embeds, using the `toml` crate the
/// project already depends on.
#[cfg(test)]
mod catalogues {
    use std::collections::{BTreeMap, BTreeSet};

    use super::Lang;

    /// One message: its dotted key, and its text per language code.
    type Entry = (String, BTreeMap<String, String>);

    /// Read every catalogue from disk rather than from a list kept here, so a
    /// newly added file cannot quietly escape these checks. `cargo test` runs
    /// with the package root as the working directory.
    fn entries() -> Vec<Entry> {
        let mut all = Vec::new();
        let dir = std::fs::read_dir("locales").expect("locales/ is missing");
        for file in dir {
            let path = file.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let parsed: toml::Value =
                toml::from_str(&raw).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            collect("", &parsed, &mut all);
        }
        assert!(!all.is_empty(), "no catalogue entries were found");
        all
    }

    /// Walk the nested tables down to the ones holding the translations.
    fn collect(prefix: &str, value: &toml::Value, out: &mut Vec<Entry>) {
        let Some(table) = value.as_table() else {
            return;
        };
        // A table whose values are strings is a message; `_version` is an
        // integer, so it never looks like one.
        let texts: BTreeMap<String, String> = table
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        if !texts.is_empty() {
            out.push((prefix.to_string(), texts));
            return;
        }
        for (key, child) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            collect(&path, child, out);
        }
    }

    /// The `%{name}` slots a text expects.
    fn placeholders(text: &str) -> BTreeSet<&str> {
        let mut found = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find("%{") {
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else { break };
            found.insert(&after[..end]);
            rest = &after[end + 1..];
        }
        found
    }

    #[test]
    fn every_message_is_translated_into_every_language() {
        for (key, texts) in entries() {
            for lang in Lang::ALL {
                let text = texts
                    .get(lang.code())
                    .unwrap_or_else(|| panic!("{key} has no {} translation", lang.code()));
                assert!(!text.trim().is_empty(), "{key}: {} is empty", lang.code());
            }
            // A stray column would be dead weight the fallback never reaches.
            assert_eq!(texts.len(), Lang::ALL.len(), "{key} has an extra language");
        }
    }

    #[test]
    fn every_translation_keeps_the_placeholders_of_the_english_text() {
        // The classic translation bug: it parses, and the path or the error the
        // slot should have carried simply vanishes from the message.
        for (key, texts) in entries() {
            let expected = placeholders(&texts["en"]);
            for (code, text) in &texts {
                assert_eq!(placeholders(text), expected, "{key} in {code}: {text}");
            }
        }
    }

    #[test]
    fn the_full_screen_hint_bar_stays_close_to_the_english_one() {
        // Unlike the picker footer, which select.rs truncates itself, this bar
        // is clipped by the terminal. English already runs to 93 columns, so
        // the budget is not a round number but the little headroom left over
        // it: a translation that needs a paragraph has to be shortened.
        let all = entries();
        let find = |key: &str| -> BTreeMap<String, String> {
            all.iter()
                .find(|(k, _)| k == key)
                .unwrap_or_else(|| panic!("{key} is missing"))
                .1
                .clone()
        };
        let bar = find("tui.hints_choosing");
        for tab_key in ["tui.tab_to_edit", "tui.tab_to_copy"] {
            let tab = find(tab_key);
            for lang in Lang::ALL {
                let code = lang.code();
                let text = bar[code].replace("%{tab}", &tab[code]);
                let width = text.chars().count();
                assert!(
                    width <= 100,
                    "{code} with {tab_key}: {width} columns, {text}"
                );
            }
        }
    }

    #[test]
    fn identifiers_are_never_translated() {
        // These are things the user has to type back verbatim; translating one
        // turns a fix-it hint into an instruction that does not work.
        const VERBATIM: &[&str] = &[
            "base_url",
            "json_mode",
            "timeout_secs",
            "send_cwd",
            "PLZ_API_KEY",
            "PLZ_CONFIG",
            "PLZ_OUTPUT_FILE",
            "plz config init",
            "plz hook",
            "cmd.exe",
            "PowerShell",
            "RemoteSigned",
            "Set-ExecutionPolicy",
            "OSC 52",
            "stderr",
        ];
        for (key, texts) in entries() {
            for token in VERBATIM.iter().filter(|t| texts["en"].contains(**t)) {
                for (code, text) in &texts {
                    assert!(
                        text.contains(token),
                        "{key} in {code} lost `{token}`: {text}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_i18n::t;

    use super::*;
    use crate::testutil::locale_guard;

    /// What the current locale resolves to, for asserting on `init`.
    fn active() -> Option<Lang> {
        Lang::from_tag(&rust_i18n::locale())
    }

    #[test]
    fn a_locale_tag_names_its_language_in_the_primary_subtag() {
        // These are the exact spellings the three platforms hand out. The
        // encoding and the modifier are noise and must not block the match.
        assert_eq!(Lang::from_tag("ru_RU.UTF-8"), Some(Lang::Ru));
        assert_eq!(Lang::from_tag("es_ES@euro"), Some(Lang::Es));
        assert_eq!(Lang::from_tag("de_DE.UTF-8"), Some(Lang::De));
        // BCP 47 with a dash, which is what Windows reports.
        assert_eq!(Lang::from_tag("fr-CA"), Some(Lang::Fr));
        assert_eq!(Lang::from_tag("DE"), Some(Lang::De));
    }

    #[test]
    fn a_language_plz_does_not_ship_is_not_recognised() {
        // Real locales, not invented ones: the point is that an Italian or a
        // Finn gets English rather than something broken.
        for tag in [
            "C",
            "POSIX",
            "",
            "   ",
            "it_IT.UTF-8",
            "fi_FI",
            "tt_RU",
            "@euro",
        ] {
            assert_eq!(Lang::from_tag(tag), None, "{tag}");
        }
    }

    #[test]
    fn a_language_list_takes_the_first_supported_entry() {
        // GNU sets LANGUAGE to a priority list; reading only its head would
        // give English to someone who asked for Italian first, German second.
        assert_eq!(first_supported("it:de_DE:en_US"), Some(Lang::De));
        assert_eq!(first_supported("it,fi"), None);
    }

    #[test]
    fn each_source_overrides_the_one_below_it() {
        assert_eq!(
            resolve(Some("es"), Some("fr"), Some("ru_RU.UTF-8")),
            Lang::Es
        );
        assert_eq!(resolve(None, Some("fr"), Some("ru_RU.UTF-8")), Lang::Fr);
        assert_eq!(resolve(None, None, Some("ru_RU.UTF-8")), Lang::Ru);
    }

    #[test]
    fn asking_for_nothing_hands_the_decision_down() {
        // "auto" is the documented default of the config key, and an unset or
        // blank override is no answer either: the next source gets its turn.
        assert_eq!(resolve(None, Some("auto"), Some("ru_RU.UTF-8")), Lang::Ru);
        assert_eq!(resolve(Some(""), None, Some("de_DE")), Lang::De);
        assert_eq!(resolve(Some("  "), Some("auto"), Some("fr_FR")), Lang::Fr);
    }

    #[test]
    fn asking_for_a_language_plz_lacks_gives_english_not_the_system_one() {
        // Deferring to the OS here would answer a request for Italian with
        // whatever the machine is set to, which is neither asked for nor
        // English. An explicit request is answered, and the answer we have is
        // English.
        assert_eq!(resolve(Some("it"), Some("fr"), Some("ru_RU")), Lang::En);
        assert_eq!(resolve(None, Some("klingon"), Some("ru_RU")), Lang::En);
    }

    #[test]
    fn an_undetectable_or_unsupported_language_falls_back_to_english() {
        // The two halves of the fallback that never reach the catalogue:
        // nothing to detect, and something detected plz has no words for.
        assert_eq!(resolve(None, None, None), Lang::En);
        assert_eq!(resolve(None, None, Some("C")), Lang::En);
        assert_eq!(resolve(None, None, Some("it_IT.UTF-8")), Lang::En);
        assert_eq!(resolve(None, Some("auto"), Some("fi_FI")), Lang::En);
    }

    #[test]
    fn a_message_is_served_in_the_current_language() {
        let _guard = locale_guard();
        rust_i18n::set_locale("ru");
        assert_eq!(
            t!("errors.no_suggestions"),
            "модель не вернула ни одного варианта"
        );
    }

    #[test]
    fn a_language_without_a_catalogue_reads_the_english_one() {
        // The third route to English, and the only one the catalogue itself
        // handles: without the fallback this prints the bare key.
        let _guard = locale_guard();
        rust_i18n::set_locale("it");
        assert_eq!(
            t!("errors.no_suggestions"),
            "the model returned no suggestions"
        );
    }

    #[test]
    fn plz_lang_reaches_the_running_process() {
        // resolve() is covered above; this is the wiring around it — reading
        // the variable and handing the code to the translation layer.
        let _guard = locale_guard();
        std::env::set_var("PLZ_LANG", "es");
        init(Some("fr"));
        std::env::remove_var("PLZ_LANG");
        assert_eq!(active(), Some(Lang::Es));
    }
}
