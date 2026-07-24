//! The developer-tools dev flag: shows in-app diagnostics (the logical-size
//! readout, the renderer string) that a stranger should never see.

/// The environment variable that reveals developer diagnostics in the chrome.
pub const DEVTOOLS_VAR: &str = "BRIGHTFIELD_DEVTOOLS";

/// Whether this process wants developer diagnostics. Set [`DEVTOOLS_VAR`] to anything but `0`.
#[must_use]
pub fn enabled() -> bool {
    std::env::var_os(DEVTOOLS_VAR).is_some_and(|v| v != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag reads as documented: unset off, `0` off, anything else on.
    /// Runs as one test because the environment is process-global.
    #[test]
    fn the_flag_reads_unset_zero_and_set() {
        // Serialised within this one test; no other test reads the variable.
        std::env::remove_var(DEVTOOLS_VAR);
        assert!(!enabled());
        std::env::set_var(DEVTOOLS_VAR, "0");
        assert!(!enabled());
        std::env::set_var(DEVTOOLS_VAR, "1");
        assert!(enabled());
        std::env::remove_var(DEVTOOLS_VAR);
    }
}
