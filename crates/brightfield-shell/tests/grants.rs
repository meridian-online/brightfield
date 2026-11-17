//! The grant store: the folders the analyst granted, kept as security-scoped bookmarks.
//!
//! Every test runs in either edition's build; `needs_follows_the_edition` checks the half that
//! applies to the build it is in. The disk-image test is ignored in both, because it attaches a
//! disk image, and is the only ignored test here, so `-- --ignored` runs it alone:
//!
//! ```text
//! cargo test -p brightfield-shell --test grants
//! cargo test -p brightfield-shell --features managed-edition --test grants
//! cargo test -p brightfield-shell --features managed-edition --test grants -- --ignored
//! ```

use std::path::{Path, PathBuf};

use brightfield_shell::edition::{self, Edition};
use brightfield_shell::grants::{GrantError, Grants, GRANTS_FILE};

/// A scratch config folder and a scratch folder to grant, both on the same disk.
fn scratch() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("a scratch folder");
    let root = std::fs::canonicalize(tmp.path()).expect("the scratch folder resolves");
    let config = root.join("config");
    let granted = root.join("Protocol");
    std::fs::create_dir_all(&config).expect("a config folder");
    std::fs::create_dir_all(granted.join("models")).expect("a folder to grant");
    (tmp, config, granted)
}

/// The paths `grants.json` in `config` names.
fn saved_paths(config: &Path) -> Vec<PathBuf> {
    let text = std::fs::read_to_string(config.join(GRANTS_FILE)).expect("grants.json is written");
    let value: serde_json::Value = serde_json::from_str(&text).expect("grants.json is JSON");
    value["grants"]
        .as_array()
        .expect("a list of grants")
        .iter()
        .map(|g| PathBuf::from(g["path"].as_str().expect("a path")))
        .collect()
}

#[test]
fn a_granted_folder_covers_what_is_inside_it() {
    let (_tmp, config, granted) = scratch();
    let grants = Grants::load(&config);
    grants.grant(&granted).expect("the folder is granted");

    let inside = granted.join("models").join("b.csv");
    assert_eq!(grants.covering(&inside), Some(granted.clone()));
    assert_eq!(grants.covering(&granted), Some(granted.clone()));
    let beside = granted.with_file_name("Protocol 2").join("a.csv");
    assert_eq!(
        grants.covering(&beside),
        None,
        "a sibling with a longer name is not inside"
    );
}

#[test]
fn grants_survive_a_reload() {
    let (_tmp, config, granted) = scratch();
    Grants::load(&config)
        .grant(&granted)
        .expect("the folder is granted");

    let again = Grants::load(&config);
    assert_eq!(again.covering(&granted.join("arcform.yaml")), Some(granted));
    assert!(again.problems().is_empty(), "{:?}", again.problems());
}

#[test]
fn a_moved_folder_is_followed_and_resaved() {
    let (_tmp, config, granted) = scratch();
    Grants::load(&config)
        .grant(&granted)
        .expect("the folder is granted");

    let moved = granted.with_file_name("Renamed in Finder");
    std::fs::rename(&granted, &moved).expect("the folder is renamed");

    let again = Grants::load(&config);
    assert_eq!(again.covering(&moved.join("x")), Some(moved.clone()));
    assert_eq!(again.covering(&granted.join("x")), None);
    assert_eq!(saved_paths(&config), vec![moved]);
}

#[test]
fn a_deleted_folder_is_dropped_and_reported() {
    let (_tmp, config, granted) = scratch();
    Grants::load(&config)
        .grant(&granted)
        .expect("the folder is granted");
    std::fs::remove_dir_all(&granted).expect("the folder is deleted");

    let again = Grants::load(&config);
    assert_eq!(again.problems(), vec![GrantError::Unresolvable(granted)]);
    assert!(saved_paths(&config).is_empty());
}

#[test]
fn an_unreadable_store_is_set_aside_not_overwritten() {
    let (_tmp, config, granted) = scratch();
    std::fs::write(config.join(GRANTS_FILE), "{ not json").expect("a broken store");

    let grants = Grants::load(&config);
    assert!(
        matches!(grants.problems().as_slice(), [GrantError::Io(_)]),
        "{:?}",
        grants.problems()
    );
    grants.grant(&granted).expect("the folder is granted");
    assert_eq!(
        std::fs::read_to_string(config.join("grants.json.unreadable")).expect("set aside"),
        "{ not json"
    );
    assert_eq!(saved_paths(&config), vec![granted]);
}

#[test]
fn a_hold_lasts_until_its_guard_drops() {
    let (_tmp, config, granted) = scratch();
    let grants = Grants::load(&config);
    grants.grant(&granted).expect("the folder is granted");

    let first = grants.hold(std::slice::from_ref(&granted));
    let second = grants.hold(std::slice::from_ref(&granted));
    assert_eq!(grants.held(), vec![granted.clone()]);
    drop(first);
    assert_eq!(grants.held(), vec![granted], "the second hold still stands");
    drop(second);
    assert!(grants.held().is_empty());
}

/// What `needs` asks for depends on the edition, so each build checks its own half: the full
/// edition reads any path and asks for nothing, and the managed one asks for each folder no grant
/// covers, once.
#[test]
fn needs_follows_the_edition() {
    let (_tmp, config, granted) = scratch();
    let elsewhere = granted.with_file_name("Sources");
    std::fs::create_dir_all(&elsewhere).expect("a second folder");
    std::fs::write(elsewhere.join("a.csv"), "x\n1\n").expect("a source");
    let grants = Grants::load(&config);
    grants.grant(&granted).expect("the folder is granted");

    let asked = grants.needs(&[
        granted.join("models").join("m.sql"),
        elsewhere.join("a.csv"),
        elsewhere.join("b.csv"),
    ]);
    match edition::current() {
        Edition::Full => assert!(asked.is_empty(), "the full edition asked for {asked:?}"),
        Edition::Managed => assert_eq!(asked, vec![elsewhere]),
    }
}

/// A disk image attached at `mount`, detached when dropped.
#[cfg(target_os = "macos")]
struct Attached {
    image: PathBuf,
    mount: PathBuf,
    attached: bool,
}

#[cfg(target_os = "macos")]
impl Attached {
    fn create(dir: &Path) -> Self {
        let image = dir.join("drive.dmg");
        let made = std::process::Command::new("hdiutil")
            .args(["create", "-quiet", "-size", "4m", "-fs", "HFS+", "-volname"])
            .arg("GrantTest")
            .arg(&image)
            .status()
            .expect("hdiutil runs");
        assert!(made.success(), "hdiutil create");
        let mut drive = Self {
            image,
            mount: dir.join("mnt"),
            attached: false,
        };
        drive.attach();
        drive
    }

    fn attach(&mut self) {
        let ok = std::process::Command::new("hdiutil")
            .args(["attach", "-quiet", "-nobrowse", "-mountpoint"])
            .arg(&self.mount)
            .arg(&self.image)
            .status()
            .expect("hdiutil runs");
        assert!(ok.success(), "hdiutil attach");
        self.attached = true;
    }

    fn detach(&mut self) {
        let ok = std::process::Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.mount)
            .status()
            .expect("hdiutil runs");
        assert!(ok.success(), "hdiutil detach");
        self.attached = false;
    }
}

#[cfg(target_os = "macos")]
impl Drop for Attached {
    fn drop(&mut self) {
        if self.attached {
            let _ = std::process::Command::new("hdiutil")
                .args(["detach", "-quiet", "-force"])
                .arg(&self.mount)
                .status();
        }
    }
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "attaches a disk image"]
fn an_unmounted_volume_is_reported_not_dropped() {
    let (tmp, config, _granted) = scratch();
    let root = std::fs::canonicalize(tmp.path()).expect("the scratch folder resolves");
    let mut drive = Attached::create(&root);
    let on_drive = std::fs::canonicalize(&drive.mount)
        .expect("the drive is mounted")
        .join("Protocol");
    std::fs::create_dir_all(&on_drive).expect("a folder on the drive");
    Grants::load(&config)
        .grant(&on_drive)
        .expect("the folder on the drive is granted");

    drive.detach();
    let unplugged = Grants::load(&config);
    assert_eq!(
        unplugged.problems(),
        vec![GrantError::Unmounted(on_drive.clone())]
    );
    assert_eq!(
        unplugged.unreachable(&on_drive.join("arcform.yaml")),
        Some(GrantError::Unmounted(on_drive.clone()))
    );
    assert_eq!(unplugged.covering(&on_drive.join("arcform.yaml")), None);
    assert_eq!(
        saved_paths(&config),
        vec![on_drive.clone()],
        "the grant is kept"
    );

    drive.attach();
    assert_eq!(
        unplugged.covering(&on_drive.join("arcform.yaml")),
        Some(on_drive.clone()),
        "once the drive is back, the same store opens it"
    );
    assert_eq!(unplugged.unreachable(&on_drive), None);
}
