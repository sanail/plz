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
