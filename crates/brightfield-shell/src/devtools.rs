//! The developer-tools dev flag: shows in-app diagnostics (the logical-size
//! readout, the renderer string) that a stranger should never see.

/// The environment variable that reveals developer diagnostics in the chrome.
pub const DEVTOOLS_VAR: &str = "BRIGHTFIELD_DEVTOOLS";

/// Whether this process wants developer diagnostics. Set [`DEVTOOLS_VAR`] to anything but `0`.
#[must_use]
pub fn enabled() -> bool {
    std::env::var_os(DEVTOOLS_VAR).is_some_and(|v| v != "0")
}
