//! Slint's testing backend: installing it, pumping its event loop, waiting on the app.
//!
//! The backend is initialised with mock time: timers and animations advance only through
//! [`pump`], while work done by the app's own threads (loads, solves, searches) runs in real
//! time and hands its results back through the event queue, which [`pump`] drains. The scale
//! factor is always 1, so logical and physical pixels are the same.

use crate::ElementHandle;
use std::{
    sync::Once,
    time::{Duration, Instant},
};

static INIT: Once = Once::new();

/// Installs the testing backend with mock time, before any window exists; the runner calls it
/// after the profile is in place. Slint accepts one platform per process, so later calls (a
/// second `Harness::run` in the same process, as the self-tests do) do nothing.
///
/// # Errors
///
/// Returns an error when another platform was already installed (the app selected a backend
/// itself); the run cannot continue then.
pub fn init_backend() -> Result<(), String> {
    std::panic::catch_unwind(|| {
        INIT.call_once(i_slint_backend_testing::init_integration_test_with_mock_time);
    })
    .map_err(|_| {
        "the Slint platform was already set before the harness could install the testing \
         backend. An app driven by the harness must not call BackendSelector::select or \
         slint::platform::set_platform in the code the harness runs: keep the backend choice \
         in the binary's main, not in the window constructor the scenarios launch"
            .to_owned()
    })
}

/// Advances mock time by `ms` (firing due timers and animations) and then runs every event the
/// app's threads have queued so far.
pub fn pump(ms: u64) {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
    drain_events();
}

/// Runs the events already in the queue: a quit is queued behind them, so the loop returns as
/// soon as it reaches it.
pub fn drain_events() {
    if slint::quit_event_loop().is_err() {
        return;
    }
    let _ = slint::run_event_loop();
}

/// Pumps (10 ms of mock time, then a short real-time sleep so worker threads get to run) until
/// `condition` holds or `timeout` of real time has passed. Returns whether it held.
pub fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        pump(10);
        if condition() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pumps for `real_ms` of real time in 10 ms mock steps; lets background work settle.
pub fn settle(real_ms: u64) {
    let _ = wait_until(Duration::from_millis(real_ms), || false);
}

/// Returns the explanation, with the three ways to switch debug info on, when the Slint build
/// carries no element debug info (then the accessibility tree cannot be walked and every query
/// would silently return nothing). The testing backend reports no type name for any element
/// without debug info, while with it the root always has one, even in an empty window.
///
/// # Errors
///
/// The explanation text, for the caller to turn into an abort request.
pub fn require_debug_info(root: &ElementHandle) -> Result<(), String> {
    if root.type_name().is_none() {
        return Err(
            "the window exposes no elements, so the UI was compiled without Slint debug info, \
             which the element API needs. Switch it on in one of three ways and rebuild: (1) in \
             build.rs call `.with_debug_info(true)` on the `slint_build::CompilerConfiguration` \
             when your `ui-harness` feature is on, and run `cargo test --features ui-harness \
             --test ui`; (2) set SLINT_EMIT_DEBUG_INFO=1 in the environment while compiling \
             (the `slint!` macro and slint-build both read it); (3) put `[env] \
             SLINT_EMIT_DEBUG_INFO = \"1\"` into .cargo/config.toml. If the window still shows \
             no elements, `cargo clean -p <your crate>` so the UI is compiled again."
                .to_owned(),
        );
    }
    Ok(())
}
