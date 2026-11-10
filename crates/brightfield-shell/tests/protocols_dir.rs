//! The folder new Protocols are made in, and the names they take.

use std::collections::HashSet;

use brightfield_shell::protocols_dir::{claim, ClaimError};

#[test]
fn first_protocol_takes_the_bare_name() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, name) = claim(tmp.path(), "earthquakes_2024").unwrap();
    assert_eq!(dir, tmp.path().join("earthquakes_2024"));
    assert_eq!(name, "earthquakes_2024");
    assert!(dir.is_dir());
}

#[test]
fn a_second_takes_dash_two() {
    let tmp = tempfile::tempdir().unwrap();
    claim(tmp.path(), "earthquakes_2024").unwrap();
    let (dir, name) = claim(tmp.path(), "earthquakes_2024").unwrap();
    assert_eq!(name, "earthquakes_2024-2");
    assert_eq!(dir, tmp.path().join("earthquakes_2024-2"));
}

#[test]
fn a_missing_parent_is_made() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("Documents").join("Protocols");
    let (dir, _) = claim(&parent, "sales").unwrap();
    assert_eq!(dir, parent.join("sales"));
}

#[test]
fn two_claims_at_once_never_share_a_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().to_path_buf();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let parent = parent.clone();
            std::thread::spawn(move || claim(&parent, "same").unwrap().0)
        })
        .collect();
    let dirs: HashSet<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(dirs.len(), 8);
}

#[cfg(unix)]
#[test]
fn a_refused_parent_makes_nothing_and_says_how() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("locked");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = claim(&parent, "sales");
    let made = std::fs::read_dir(&parent).unwrap().count();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    match result {
        Err(ClaimError::Refused(p)) => {
            assert_eq!(p, parent);
            assert!(ClaimError::Refused(p)
                .to_string()
                .contains("Privacy & Security"));
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    assert_eq!(made, 0, "no folder is made");
}
