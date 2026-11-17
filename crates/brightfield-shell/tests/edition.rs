//! The edition a build is, and what each edition allows.
//!
//! The default build is the full edition; `--features managed-edition` builds
//! the managed one. The two `current_is_*` tests each run in one of those
//! builds and are ignored in the other, so either command runs every test that
//! applies to its build:
//!
//! ```text
//! cargo test -p brightfield-shell --test edition
//! cargo test -p brightfield-shell --features managed-edition --test edition
//! ```

use brightfield_shell::edition::{self, Capability, Edition};

#[test]
fn full_allows_everything_managed_allows_nothing_on_the_list() {
    for &c in Capability::ALL {
        assert!(Edition::Full.allows(c), "the full edition refuses {c:?}");
        assert!(
            !Edition::Managed.allows(c),
            "the managed edition allows {c:?}"
        );
    }
}

/// `Capability::ALL` names every capability once. A capability left out of it
/// would escape the test above; the match fails to compile when a variant is
/// added, which sends the person adding it here.
#[test]
fn the_capability_list_is_complete() {
    let listed = |c: Capability| Capability::ALL.iter().filter(|&&a| a == c).count();
    for &c in Capability::ALL {
        match c {
            Capability::CommandSteps
            | Capability::OutsidePrograms
            | Capability::ExtensionInstall
            | Capability::LauncherCommand
            | Capability::UngrantedPaths => assert_eq!(listed(c), 1, "{c:?}"),
        }
    }
    assert_eq!(Capability::ALL.len(), 5);
}

#[test]
fn each_edition_has_a_name_of_its_own() {
    let full = Edition::Full.name();
    let managed = Edition::Managed.name();
    assert!(!full.trim().is_empty());
    assert!(!managed.trim().is_empty());
    assert_ne!(full, managed);
}

#[test]
fn the_full_edition_is_on_meridians_site() {
    assert_eq!(
        edition::full_edition_url(),
        "https://meridian.online/brightfield"
    );
}

#[test]
#[cfg_attr(feature = "managed-edition", ignore = "a managed-edition build")]
fn current_is_full_without_the_feature() {
    assert_eq!(edition::current(), Edition::Full);
}

#[test]
#[cfg_attr(
    not(feature = "managed-edition"),
    ignore = "run with --features managed-edition"
)]
fn current_is_managed_with_the_feature() {
    assert_eq!(edition::current(), Edition::Managed);
}
