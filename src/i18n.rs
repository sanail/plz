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
/// anything on disk. Whatever is undetectable or unsupported means English.
fn resolve(forced: Option<&str>, detected: Option<&str>) -> Lang {
    forced
        .and_then(first_supported)
        .or_else(|| detected.and_then(first_supported))
        .unwrap_or(Lang::En)
}

/// Resolve the interface language and apply it to the whole process.
///
/// `PLZ_LANG` overrides the OS, in the same shape as `PLZ_API_KEY` and
/// `PLZ_CONFIG`. The OS answer comes from the platform rather than from `LANG`:
/// macOS reports the region there, and Windows sets it at all only under MSYS2.
pub fn init() {
    let forced = std::env::var("PLZ_LANG").ok();
    let detected = sys_locale::get_locale();
    rust_i18n::set_locale(resolve(forced.as_deref(), detected.as_deref()).code());
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
    fn an_override_wins_over_the_detected_language() {
        assert_eq!(resolve(Some("es"), Some("ru_RU.UTF-8")), Lang::Es);
        // An unsupported override is not a veto on the OS: it is simply not an
        // answer, so detection still gets its turn.
        assert_eq!(resolve(Some("it"), Some("ru_RU.UTF-8")), Lang::Ru);
    }

    #[test]
    fn an_undetectable_or_unsupported_language_falls_back_to_english() {
        // The two halves of the fallback that never reach the catalogue:
        // nothing to detect, and something detected plz has no words for.
        assert_eq!(resolve(None, None), Lang::En);
        assert_eq!(resolve(None, Some("C")), Lang::En);
        assert_eq!(resolve(None, Some("it_IT.UTF-8")), Lang::En);
        assert_eq!(resolve(Some(""), Some("fi_FI")), Lang::En);
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
        init();
        std::env::remove_var("PLZ_LANG");
        assert_eq!(active(), Some(Lang::Es));
    }
}
