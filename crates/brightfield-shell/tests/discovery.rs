//! The discovery seam: its stand-in, the full pass that runs on a worker and folds into the model
//! as it stands when it lands, starts only once the sample's save is written, and is stopped when a
//! newer pass supersedes it, and the dovetail child that is not built in yet.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use brightfield_ossie::{DecidedBy, Edit, Model, Role};
use brightfield_shell::data_file::{self, Opened};
use brightfield_shell::discovery::{proposal_file, Cancel, Discovery, Dovetail, Pass, StandIn};
use brightfield_shell::model_inspector::ModelAction;
use brightfield_shell::model_pane::{Dispatched, ModelPane};
use brightfield_shell::own_files::{self, OwnProtocol};
use brightfield_shell::run::Runner;

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        root
    })
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../brightfield-ossie/fixtures/retail_extract")
}

#[test]
fn stand_in_reads_fixture_model() {
    let dir = scratch().join("input").join("fixture-protocol");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        fixture().join("model.ossie.yaml"),
        dir.join("model.ossie.yaml"),
    )
    .unwrap();
    let protocol = OwnProtocol {
        dir,
        sources: Default::default(),
        profiles: Vec::new(),
    };
    for pass in [Pass::Sample, Pass::Full] {
        let model = StandIn
            .discover(&protocol, pass)
            .expect("the fixture reads");
        assert_eq!(model.datasets.len(), 8);
        assert_eq!(model.relationships.len(), 7);
    }
}

#[test]
fn stand_in_without_model_lists_unclear_tables() {
    let root = scratch().join("input").join("plain");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("customers.csv"), "id,name\n1,Ada\n2,Bo\n").unwrap();
    std::fs::write(root.join("orders.csv"), "id,customer_id\n10,1\n").unwrap();
    let mut protocol = own_files::create(&root).unwrap();
    own_files::run_loads(
        &protocol,
        &Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")),
    )
    .unwrap();
    own_files::profile(&mut protocol).unwrap();

    let model = StandIn.discover(&protocol, Pass::Sample).unwrap();
    let names: Vec<&str> = model.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["customers", "orders"]);
    assert!(model.relationships.is_empty());
    for d in &model.datasets {
        assert_eq!(d.meta.role, Role::Unclear);
        assert_eq!(d.meta.decided_by, DecidedBy::Dovetail);
        assert_eq!(d.source, format!("raw_{}", d.name));
        assert!(d.fields.iter().all(|f| !f.name.starts_with("__source")));
        assert!(d.meta.rows.is_some());
        assert!(d.meta.source_file.ends_with(&format!("{}.csv", d.name)));
    }
    let customers = &model.datasets[0];
    let fields: Vec<&str> = customers.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(fields, ["id", "name"]);
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// Held around each open that makes a Protocol folder.
fn folder_lock() -> MutexGuard<'static, ()> {
    static OPENS: Mutex<()> = Mutex::new(());
    OPENS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The model written for the retail extract.
fn retail_model() -> Model {
    let text = std::fs::read_to_string(fixture().join("model.ossie.yaml")).unwrap();
    brightfield_ossie::read(&text).unwrap()
}

/// The retail extract's CSVs, without its model, copied into a folder named `name`.
fn retail_copy(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(fixture()).unwrap().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.ends_with(".csv") {
            std::fs::copy(entry.path(), dir.join(&file)).unwrap();
        }
    }
    dir
}

/// A discoverer whose sample pass answers `sample` at once, and whose full pass waits for the
/// test to hand it a model, then says on `asked` whether it had been asked to stop by then.
struct Gated {
    sample: Model,
    full: Receiver<Model>,
    asked: Option<Sender<bool>>,
}

impl Gated {
    fn new(sample: Model, full: Receiver<Model>) -> Self {
        Self {
            sample,
            full,
            asked: None,
        }
    }
}

impl Discovery for Gated {
    fn discover(&self, protocol: &OwnProtocol, pass: Pass) -> Result<Model, String> {
        self.discover_until(protocol, pass, &Cancel::default())
    }

    fn discover_until(
        &self,
        _protocol: &OwnProtocol,
        pass: Pass,
        cancel: &Cancel,
    ) -> Result<Model, String> {
        match pass {
            Pass::Sample => Ok(self.sample.clone()),
            Pass::Full => {
                let model = self
                    .full
                    .recv()
                    .map_err(|_| "the test let the full pass go".to_string())?;
                if let Some(asked) = &self.asked {
                    let _ = asked.send(cancel.requested());
                }
                Ok(model)
            }
        }
    }
}

/// A discoverer whose sample pass answers `sample`, and whose full pass reads the model file as
/// it finds it on disk and sends what it read to the test.
struct ReadsTheFile {
    sample: Model,
    read: Sender<Result<Model, String>>,
}

impl Discovery for ReadsTheFile {
    fn discover(&self, protocol: &OwnProtocol, pass: Pass) -> Result<Model, String> {
        match pass {
            Pass::Sample => Ok(self.sample.clone()),
            Pass::Full => {
                let text = std::fs::read_to_string(protocol.dir.join("model.ossie.yaml"))
                    .map_err(|e| e.to_string());
                let read =
                    text.and_then(|t| brightfield_ossie::read(&t).map_err(|e| e.to_string()));
                let _ = self.read.send(read.clone());
                read
            }
        }
    }
}

/// The retail model with what Dovetail decided of stores changed: a reference table.
fn stores_as_reference() -> Model {
    let mut model = retail_model();
    for d in &mut model.datasets {
        if d.name == "stores" {
            d.meta.role = Role::Reference;
        }
    }
    model
}

/// The retail extract copied into `name`, opened as a Protocol, with its written model saved.
fn retail_pane(name: &str) -> ModelPane {
    let opened = {
        let _held = folder_lock();
        data_file::open_any(&retail_copy(name), &runner()).expect("it opens")
    };
    let Opened::Model(protocol) = opened else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");
    pane.save_model(retail_model())
        .expect("the written model runs");
    pane
}

/// What the analyst decides between the passes, as the pane's model holds it: the possible
/// segments join kept, and promo_codes a reference table.
fn analyst_decisions_stand(model: &Model) {
    let join = model
        .relationships
        .iter()
        .find(|r| r.name == "customer_segments_customers")
        .expect("the kept join is still there");
    assert_eq!(join.meta.decided_by, DecidedBy::Analyst);
    let promo = model
        .datasets
        .iter()
        .find(|d| d.name == "promo_codes")
        .unwrap();
    assert_eq!(promo.meta.role, Role::Reference);
    assert_eq!(promo.meta.decided_by, DecidedBy::Analyst);
}

fn role_of(model: &Model, name: &str) -> Role {
    model
        .datasets
        .iter()
        .find(|d| d.name == name)
        .unwrap()
        .meta
        .role
}

#[test]
fn sample_then_full_merges_without_flicker() {
    let opened = {
        let _held = folder_lock();
        data_file::open_any(&retail_copy("gated"), &runner()).expect("it opens")
    };
    let Opened::Model(protocol) = opened else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");
    pane.save_model(retail_model())
        .expect("the written model runs");

    // The sample pass proposes stores as a reference table, and its full pass waits.
    let (release, gate) = mpsc::channel();
    let done = pane.rediscover(Gated::new(stores_as_reference(), gate));
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    assert!(pane.discovering(), "the full pass is still out");
    let after_sample = pane.model.clone();
    let segments = after_sample
        .relationships
        .iter()
        .find(|r| r.name == "customer_segments_customers")
        .unwrap();
    assert_eq!(segments.meta.decided_by, DecidedBy::Dovetail);
    assert_eq!(role_of(&after_sample, "promo_codes"), Role::Unclear);
    assert_eq!(
        role_of(&after_sample, "stores"),
        Role::Reference,
        "the sample was folded in"
    );

    // The analyst keeps the possible join and calls promo_codes a reference table while the
    // full pass runs.
    for edit in [
        Edit::KeepJoin {
            relationship: "customer_segments_customers".into(),
        },
        Edit::SetRole {
            dataset: "promo_codes".into(),
            role: Role::Reference,
        },
    ] {
        let done = pane.dispatch(ModelAction::Edit(edit));
        assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    }
    analyst_decisions_stand(&pane.model);
    assert_eq!(pane.poll_discovery(), None, "the full pass has not landed");
    analyst_decisions_stand(&pane.model);

    // The full pass lands with a proposal that would undo both, and that changes what Dovetail
    // decided besides: customers becomes a snapshot.
    let mut full = stores_as_reference();
    full.relationships
        .retain(|r| r.name != "customer_segments_customers");
    for d in &mut full.datasets {
        match d.name.as_str() {
            "promo_codes" => d.meta.role = Role::Event,
            "customers" => d.meta.role = Role::Snapshot,
            _ => {}
        }
    }
    release.send(full).unwrap();
    let due = Instant::now() + Duration::from_secs(30);
    let landed = loop {
        // Each poll is a frame: none shows the analyst's decisions undone.
        let polled = pane.poll_discovery();
        analyst_decisions_stand(&pane.model);
        if let Some(landed) = polled {
            break landed;
        }
        assert!(Instant::now() < due, "the full pass never landed");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(matches!(landed, Dispatched::Done(_)), "{landed:?}");
    assert!(!pane.discovering());

    // Full folded into the model the analyst left, and that is what the file holds.
    analyst_decisions_stand(&pane.model);
    assert_eq!(role_of(&pane.model, "customers"), Role::Snapshot);
    assert_eq!(role_of(&pane.model, "stores"), Role::Reference);
    let text = std::fs::read_to_string(pane.protocol.dir.join("model.ossie.yaml")).unwrap();
    assert_eq!(pane.model, brightfield_ossie::read(&text).unwrap());
}

#[test]
fn full_pass_waits_for_a_file_edit_to_be_read() {
    let opened = {
        let _held = folder_lock();
        data_file::open_any(&retail_copy("gated-edit"), &runner()).expect("it opens")
    };
    let Opened::Model(protocol) = opened else {
        panic!("several tables are a model");
    };
    let mut pane = ModelPane::open(protocol, runner()).expect("the model reads");
    pane.save_model(retail_model())
        .expect("the written model runs");
    let (release, gate) = mpsc::channel();
    pane.rediscover(Gated::new(retail_model(), gate));

    // A hand edit is typed and waiting out its pause when the full pass lands.
    let text = pane.file_text().to_string();
    let promo = r#"data: '{"role":"unclear""#;
    assert_eq!(text.matches(promo).count(), 1, "{text}");
    pane.set_file_draft(text.replacen(promo, r#"data: '{"role":"reference""#, 1));
    let mut full = retail_model();
    for d in &mut full.datasets {
        if d.name == "promo_codes" {
            d.meta.role = Role::Event;
        }
    }
    release.send(full).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(pane.poll_discovery(), None, "the edit is read first");
    assert!(pane.discovering());

    // Once the edit is read, the full pass folds in around it.
    assert_eq!(pane.tick_file_edit(0), None);
    assert_eq!(pane.tick_file_edit(10_000), Some(Ok(())));
    let due = Instant::now() + Duration::from_secs(30);
    while pane.poll_discovery().is_none() {
        assert!(Instant::now() < due, "the full pass never landed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(role_of(&pane.model, "promo_codes"), Role::Reference);
}

#[test]
#[ignore = "dovetail is not built into Brightfield until dovetail's DuckDB 2.0 release ships, \
            with Ossie output, table roles and the sample and full passes"]
fn dovetail_child_writes_valid_ossie() {
    let root = retail_copy("dovetail");
    let mut protocol = {
        let _held = folder_lock();
        let protocol = own_files::create(&root).unwrap();
        own_files::run_loads(&protocol, &runner()).unwrap();
        protocol
    };
    own_files::profile(&mut protocol).unwrap();
    own_files::write_packages(&protocol).unwrap();
    // The model the analyst holds, which the child must leave alone.
    let model_file = protocol.dir.join("model.ossie.yaml");
    let held = brightfield_ossie::write(&StandIn.discover(&protocol, Pass::Sample).unwrap());
    std::fs::write(&model_file, &held).unwrap();

    for pass in [Pass::Sample, Pass::Full] {
        let answered = Dovetail
            .discover(&protocol, pass)
            .expect("the dovetail child runs");
        let text = std::fs::read_to_string(proposal_file(&protocol.dir, pass))
            .expect("the child wrote its proposal file");
        let issues = brightfield_ossie::validate(&text);
        assert!(issues.is_empty(), "{issues:?}");
        let proposed = brightfield_ossie::read(&text).expect("Brightfield reads it");
        assert_eq!(answered, proposed, "the answer is the file the child wrote");
        let names: Vec<&str> = proposed.datasets.iter().map(|d| d.name.as_str()).collect();
        for table in &protocol.sources.tables {
            assert!(
                names.contains(&table.name.as_str()),
                "{} is modelled",
                table.name
            );
        }
        assert!(proposed
            .datasets
            .iter()
            .all(|d| d.meta.decided_by == DecidedBy::Dovetail));
        assert!(
            !proposed.relationships.is_empty(),
            "the retail joins are found"
        );
        assert_eq!(
            std::fs::read_to_string(&model_file).unwrap(),
            held,
            "the child never writes model.ossie.yaml"
        );
    }
}

#[test]
fn full_pass_reads_the_file_the_sample_save_wrote() {
    let mut pane = retail_pane("reads-after-save");
    assert_eq!(role_of(&pane.model, "stores"), Role::Entity);
    let (read, seen) = mpsc::channel();
    let done = pane.rediscover(ReadsTheFile {
        sample: stores_as_reference(),
        read,
    });
    assert!(matches!(done, Dispatched::Done(_)), "{done:?}");
    assert_eq!(role_of(&pane.model, "stores"), Role::Reference);

    // The full pass started once the sample's save was written: it reads the whole new file.
    let found = seen
        .recv_timeout(Duration::from_secs(30))
        .expect("the full pass ran")
        .expect("the file it read was whole");
    assert_eq!(found, pane.model, "it read the file the save wrote");
    assert!(
        !pane.protocol.dir.join(".model.ossie.yaml.writing").exists(),
        "no half-written file is left beside it"
    );
}

#[test]
fn a_superseded_full_pass_is_stopped_and_never_merged() {
    let mut pane = retail_pane("superseded");
    let (release_old, old_gate) = mpsc::channel();
    let (asked, old_asked) = mpsc::channel();
    pane.rediscover(Gated {
        asked: Some(asked),
        ..Gated::new(retail_model(), old_gate)
    });
    let (release_new, new_gate) = mpsc::channel();
    pane.rediscover(Gated::new(retail_model(), new_gate));

    // The first pass answers after the second superseded it: it was asked to stop, and what it
    // proposes goes nowhere.
    release_old.send(stores_as_reference()).unwrap();
    assert_eq!(
        old_asked.recv_timeout(Duration::from_secs(30)),
        Ok(true),
        "the superseded pass was asked to stop"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(pane.poll_discovery(), None, "only the newer pass can land");
    assert_eq!(role_of(&pane.model, "stores"), Role::Entity);

    release_new.send(retail_model()).unwrap();
    let due = Instant::now() + Duration::from_secs(30);
    while pane.poll_discovery().is_none() {
        assert!(Instant::now() < due, "the newer pass never landed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!pane.discovering());
    assert_eq!(role_of(&pane.model, "stores"), Role::Entity);
}
