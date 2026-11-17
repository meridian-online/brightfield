//! Which edition of Brightfield this build is, and what that edition may do.
//!
//! Brightfield ships from one code base in more than one edition. The full
//! edition, from Homebrew, the install command and the signed download, runs
//! everything. The managed edition, for the Mac App Store, runs under App
//! Sandbox and leaves out what App Review or a managed Mac would refuse:
//! running programs that are not in the app, installing DuckDB extensions from
//! the network, and reading a path the analyst has not granted.
//!
//! The edition is chosen at build time: a build with the cargo feature
//! `managed-edition` is the managed edition, and every other build is the full
//! one. This module is the one place that reads the feature. Code elsewhere
//! asks [`current`] whether a [`Capability`] is allowed, rather than testing
//! the feature itself, so a third edition is a new [`Edition`] value and a row
//! in [`Edition::allows`], not a fork of every call site. A crate below the
//! shell, such as the engine, cannot read this module without depending on the
//! shell, so the shell passes it the answer as an argument rather than the
//! crate gaining a feature of its own.
//!
//! The editions' names are working names while their final names are chosen.
//! An edition's name as an analyst reads it comes from [`Edition::name`], so a
//! rename changes this file and no other; the variant identifiers are internal.

/// An edition of Brightfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    /// Unsandboxed: runs command steps, the analyst's own programs and any
    /// DuckDB extension.
    Full,
    /// Sandboxed for the Mac App Store: runs arc's own operators and the DuckDB
    /// extensions built into the app, and reads only what the analyst grants.
    Managed,
}

/// Something one edition may do and another may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Run a Protocol's `command:` steps and shell preconditions.
    CommandSteps,
    /// Run arc operators that start a program from outside the app.
    OutsidePrograms,
    /// Install or load a DuckDB extension that is not built into the app.
    ExtensionInstall,
    /// Install the launcher command that opens Brightfield from a terminal.
    LauncherCommand,
    /// Read a path that no folder grant covers.
    UngrantedPaths,
}

impl Capability {
    /// Every capability, once each.
    pub const ALL: &'static [Capability] = &[
        Capability::CommandSteps,
        Capability::OutsidePrograms,
        Capability::ExtensionInstall,
        Capability::LauncherCommand,
        Capability::UngrantedPaths,
    ];
}

impl Edition {
    /// Whether this edition may do `capability`.
    pub fn allows(self, capability: Capability) -> bool {
        match (self, capability) {
            (Edition::Full, _) => true,
            (
                Edition::Managed,
                Capability::CommandSteps
                | Capability::OutsidePrograms
                | Capability::ExtensionInstall
                | Capability::LauncherCommand
                | Capability::UngrantedPaths,
            ) => false,
        }
    }

    /// The edition's name as an analyst reads it, in lower case for use inside
    /// a sentence ("the managed edition"). A working name for now.
    pub fn name(self) -> &'static str {
        match self {
            Edition::Full => "full",
            Edition::Managed => "managed",
        }
    }
}

/// The edition this build is: [`Edition::Managed`] exactly when it was built
/// with the `managed-edition` feature.
pub fn current() -> Edition {
    if cfg!(feature = "managed-edition") {
        Edition::Managed
    } else {
        Edition::Full
    }
}

/// Where the full edition is offered, for an edition that sends an analyst to
/// it for something it does not run.
pub fn full_edition_url() -> &'static str {
    "https://meridian.online/brightfield"
}
