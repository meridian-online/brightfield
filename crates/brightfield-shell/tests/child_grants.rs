//! Runs and join proposals hold their folders' grants for as long as their child goes, and a run
//! the system refused mid-way ends with a refusal naming the path.
//!
//! The child is a stand-in shell script, so the test can hold it at a known point: it marks that
//! it started, waits until the test lets it go, then reads its source the way a step would, and
//! fails with the system's own words when the read is refused. Every test runs in either edition's
//! build, since the hold and the refusal are the same in both:
//!
//! ```text
//! cargo test -p brightfield-shell --features managed-edition --test child_grants
//! cargo test -p brightfield-shell --test child_grants
//! ```

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use brightfield_shell::discovery::{Discovery, Dovetail, Pass};
use brightfield_shell::grants;
use brightfield_shell::own_files::OwnProtocol;
use brightfield_shell::refusal::{refusal_in_run, RefusedDuring};
use brightfield_shell::run::{Run, Runner};

/// This file's scratch folder, with the config folder, the Protocols folder and arc's folders in
/// it, so the process's grant store is `config/grants.json` here and nothing is written under the
/// home directory.
fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-child-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        let root = std::fs::canonicalize(&root).expect("the scratch folder resolves");
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        grants::store()
            .grant(&root.join("protocols"))
            .expect("the Protocols folder is granted");
        root
    })
}

/// A new empty folder of this test's, under `input`.
fn folder(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

/// A Protocol in `dir` whose record of sources names `sources`, each a CSV that exists, with every
/// folder involved granted.
fn protocol_in(dir: &Path, sources: &[PathBuf]) {
    std::fs::create_dir_all(dir).expect("the Protocol folder");
    std::fs::write(dir.join("arcform.yaml"), "name: held\n").expect("its manifest");
    let tables: Vec<serde_json::Value> = sources
        .iter()
        .enumerate()
        .map(|(i, csv)| {
            std::fs::write(csv, "a\n1\n").expect("a source");
            serde_json::json!({ "name": format!("t{i}"), "file": csv.to_string_lossy(), "reader": "csv" })
        })
        .collect();
    let record = serde_json::json!({
        "dir": dir.to_string_lossy(),
        "tables": tables,
        "skipped": [],
    });
    std::fs::write(dir.join(".sources.json"), record.to_string()).expect("its sources");
    grants::store().grant(dir).expect("the Protocol is granted");
    for csv in sources {
        grants::store()
            .grant(csv.parent().expect("a folder"))
            .expect("the source's folder is granted");
    }
}

/// A stand-in child at `gate/child.sh`: it marks `gate/started`, waits for `gate/go`, then reads
/// each of `reads` and exits 0 only when every read succeeds.
fn child(gate: &Path, reads: &[PathBuf]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = gate.join("child.sh");
    let mut body = format!(
        "#!/bin/sh\ntouch '{g}/started'\nwhile [ ! -e '{g}/go' ]; do sleep 0.05; done\n",
        g = gate.display()
    );
    for read in reads {
        body.push_str(&format!("cat '{}' > /dev/null || exit 1\n", read.display()));
    }
    std::fs::write(&script, body).expect("the stand-in child");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("it is executable");
    script
}

/// Wait until the stand-in child in `gate` has started.
fn wait_for_start(gate: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !gate.join("started").exists() {
        assert!(Instant::now() < deadline, "the child never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Which of `folders` a hold stands on now.
fn held_of(folders: &[&PathBuf]) -> Vec<PathBuf> {
    let held = grants::store().held();
    folders
        .iter()
        .filter(|f| held.contains(f))
        .map(|f| (*f).clone())
        .collect()
}

#[test]
fn a_run_holds_its_folders_until_it_ends() {
    let root = folder("holds");
    let (left, right, dir, gate) = (
        root.join("left"),
        root.join("right"),
        root.join("protocol"),
        root.join("gate"),
    );
    for d in [&left, &right, &gate] {
        std::fs::create_dir_all(d).expect("a folder");
    }
    protocol_in(&dir, &[left.join("a.csv"), right.join("b.csv")]);
    let runner = Runner::at(child(&gate, &[]));

    let run = Run::begin(&runner, &dir, || {});
    wait_for_start(&gate);
    let protocols = scratch().join("protocols");
    let all = [&dir, &left, &right, &protocols];
    assert_eq!(
        held_of(&all),
        vec![dir.clone(), left.clone(), right.clone(), protocols.clone()],
        "the Protocol, both source folders and the Protocols folder are held while it runs"
    );

    std::fs::write(gate.join("go"), "").expect("the child is let go");
    let finished = run.wait();
    assert!(finished.succeeded, "the run succeeds: {}", finished.log);
    assert_eq!(
        held_of(&[&dir, &left, &right]),
        Vec::<PathBuf>::new(),
        "nothing is held once it ends"
    );
}

#[test]
fn a_run_refused_mid_way_reports_the_path() {
    use std::os::unix::fs::PermissionsExt;
    let root = folder("refused");
    let (source, dir, gate) = (
        root.join("source"),
        root.join("protocol"),
        root.join("gate"),
    );
    for d in [&source, &gate] {
        std::fs::create_dir_all(d).expect("a folder");
    }
    let csv = source.join("a.csv");
    protocol_in(&dir, std::slice::from_ref(&csv));
    let runner = Runner::at(child(&gate, std::slice::from_ref(&csv)));

    let run = Run::begin(&runner, &dir, || {});
    wait_for_start(&gate);
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000))
        .expect("the source folder is shut");
    std::fs::write(gate.join("go"), "").expect("the child is let go");
    let finished = run.wait();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755))
        .expect("the source folder is opened again");

    assert!(!finished.succeeded, "the run fails: {}", finished.log);
    let refusal = refusal_in_run(&finished).expect("the run reports a refusal");
    assert_eq!(refusal.path, csv, "it names the refused source");
    assert_eq!(refusal.during, RefusedDuring::Run);
    assert!(
        refusal.sentence().contains(&*csv.to_string_lossy()),
        "the banner's sentence names it: {}",
        refusal.sentence()
    );
}

#[test]
fn a_run_that_fails_for_another_reason_reports_no_refusal() {
    let root = folder("missing");
    let (source, dir, gate) = (
        root.join("source"),
        root.join("protocol"),
        root.join("gate"),
    );
    for d in [&source, &gate] {
        std::fs::create_dir_all(d).expect("a folder");
    }
    protocol_in(&dir, &[source.join("a.csv")]);
    std::fs::write(gate.join("go"), "").expect("the child goes at once");
    let runner = Runner::at(child(&gate, &[source.join("not-there.csv")]));

    let finished = Run::begin(&runner, &dir, || {}).wait();
    assert!(!finished.succeeded, "the run fails: {}", finished.log);
    assert_eq!(
        refusal_in_run(&finished),
        None,
        "a missing file is not a refusal"
    );
}

#[test]
fn dovetail_holds_the_folder_it_proposes_over() {
    let root = folder("dovetail");
    let (dir, gate) = (root.join("protocol"), root.join("gate"));
    std::fs::create_dir_all(&gate).expect("a folder");
    protocol_in(&dir, &[]);
    let db = arc::spec::protocol_db_path(&dir).expect("the Protocol's database path");
    std::fs::create_dir_all(db.parent().expect("its folder")).expect("its folder");
    std::fs::write(&db, b"").expect("a database to copy");
    let dovetail = Dovetail::new(&Runner::at(child(&gate, &[])));
    let protocol = OwnProtocol {
        dir: dir.clone(),
        sources: Default::default(),
        profiles: Vec::new(),
    };

    let pass = std::thread::spawn(move || dovetail.discover(&protocol, Pass::Sample));
    wait_for_start(&gate);
    assert_eq!(
        held_of(&[&dir]),
        vec![dir.clone()],
        "the Protocol is held while dovetail proposes"
    );
    std::fs::write(gate.join("go"), "").expect("the child is let go");
    let _ = pass.join().expect("the pass ends");
    assert_eq!(
        held_of(&[&dir]),
        Vec::<PathBuf>::new(),
        "and released once it ends"
    );
}
