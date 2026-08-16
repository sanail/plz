//! Helpers shared by the unit tests.

use std::sync::{Mutex, MutexGuard};

/// Serialises every test that touches a process-wide environment variable.
///
/// `cargo test` runs the whole crate's tests in one process on several threads,
/// so these tests are not isolated from each other: one that removes
/// `DEEPSEEK_API_KEY` and one that sets it will fail each other at random, and
/// the same goes for `PLZ_OUTPUT_FILE` and `PLZ_CONFIG`. The lock lives here
/// rather than in one test module because the conflicting tests sit in
/// different files.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Hold the environment lock for the rest of the test.
///
/// A poisoned lock is recovered rather than propagated: one failing test must
/// not turn into a cascade of failures in every other test that touches the
/// environment.
pub fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Holds the environment lock and puts the interface language back on drop.
///
/// The locale is process-wide too, so a test that changes it would otherwise
/// pick the language for whatever runs next. English is the value the rest of
/// the suite assumes, because nothing but a test ever calls `i18n::init`.
pub struct LocaleGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for LocaleGuard {
    fn drop(&mut self) {
        rust_i18n::set_locale("en");
    }
}

pub fn locale_guard() -> LocaleGuard {
    LocaleGuard(env_guard())
}
