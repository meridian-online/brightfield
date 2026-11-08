//! Which own-files Protocols the front door lists, and where new ones are made.

use std::path::{Path, PathBuf};

use brightfield_shell::discovery::MODEL_FILE;
use brightfield_shell::{own_files, protocols_dir, startup};

/// One scratch root for this file, with both folders pointed into it.
fn scratch() -> &'static Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-own-listed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(startup::CONFIG_DIR_VAR, root.join("config"));
        std::env::set_var(protocols_dir::PROTOCOLS_DIR_VAR, root.join("protocols"));
        root
    })
}

/// A folder under `parent` that reads as an own-files Protocol with a model.
fn protocol_in(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("arcform.yaml"),
        "name: x\nengine: duckdb\nsteps: []\n",
    )
    .unwrap();
    std::fs::write(dir.join(MODEL_FILE), "{}\n").unwrap();
    std::path::absolute(&dir).unwrap()
}

#[test]
fn listed_returns_protocols_from_the_protocols_folder_and_the_older_one() {
    let root = scratch();
    let new = protocol_in(&protocols_dir::root(), "made_now");
    let old = protocol_in(
        &startup::datasets_dir()
            .unwrap()
            .join("protocols")
            .join(own_files::OWN_DIR),
        "made_before-1",
    );
    assert!(new.starts_with(root.join("protocols")));
    let listed = own_files::listed();
    assert!(listed.contains(&new), "{listed:?}");
    assert!(listed.contains(&old), "{listed:?}");
    assert_eq!(
        listed.iter().filter(|p| **p == new).count(),
        1,
        "each is listed once"
    );
}

#[test]
fn the_environment_variable_wins_when_set_and_not_empty() {
    let root = scratch();
    assert_eq!(protocols_dir::root(), root.join("protocols"));
}
