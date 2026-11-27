//! Brightfield's operation registry against arc's own list and the key
//! registry: a slot for every operation arc holds, and a built operation
//! exactly where a key or a palette row can start it.
//!
//! The runner is the `brightfield-shell` binary cargo built for this suite,
//! which becomes `arc` under the runner variable, so the operations and their
//! descriptions here are arc's at the pinned rev.

use std::collections::BTreeSet;

use brightfield_keys::registry;
use brightfield_protocol::silver::FinetypeTransforms;
use brightfield_shell::operation::{self, OperationDescription, ValueShape};
use brightfield_shell::operation_ui::{self, Begin, OperationUi, Stage, Start, OPERATIONS};
use brightfield_shell::run::Runner;

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

fn described(long_name: &str) -> OperationDescription {
    operation::describe(&runner(), long_name)
        .unwrap_or_else(|e| panic!("{long_name} describes: {e}"))
}

/// The registry verb that starts `long_name`: its own long name, but for the
/// three operations whose command was in the registry before it.
fn verb_of(long_name: &str) -> &str {
    match long_name {
        "filter-rows" => "filter-rows-at-cursor",
        "sort-rows" => "sort-rows-at-cursor",
        "read-file" => "open-sources-browser",
        other => other,
    }
}

/// Columns of the housing sample, for a draft to begin over.
fn housing_columns() -> Vec<String> {
    ["median_income", "house_age", "rooms", "population"]
        .iter()
        .map(|c| (*c).to_string())
        .collect()
}

/// The draft `ui` begins as over the housing sample's columns, from its first
/// start, on `house_age`.
fn begun(ui: &OperationUi, description: &OperationDescription) -> operation_ui::Draft {
    let columns = housing_columns();
    let taken = vec![("by_age".to_string(), vec!["house_age".to_string()])];
    let transforms = FinetypeTransforms::vendored();
    let quote = |name: &str| name.to_string();
    let begin = Begin {
        start: ui.starts.first().copied().unwrap_or(Start::Cell),
        table: Some("california_housing_sample"),
        column: Some("house_age"),
        columns: &columns,
        facts: &[],
        description,
        taken: &taken,
        model: None,
        transforms: &transforms,
        quote: &quote,
    };
    (ui.begin)(&begin).unwrap_or_else(|e| panic!("{} begins: {e}", ui.long_name))
}

#[test]
fn every_operation_arc_holds_has_one_slot() {
    let listed: BTreeSet<String> = operation::list_operations(&runner())
        .expect("arc lists its operations")
        .into_iter()
        .collect();
    let slots: BTreeSet<String> = OPERATIONS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    assert_eq!(slots.len(), 13, "no long name twice");
    assert_eq!(slots, listed);
    for (name, slot) in OPERATIONS {
        if let Some(ui) = slot() {
            assert_eq!(ui.long_name, name, "the slot holds its own operation");
        }
    }
}

#[test]
fn an_operation_is_built_exactly_when_its_command_is_in_the_key_registry() {
    let verbs: BTreeSet<&str> = registry().iter().map(|v| v.longname).collect();
    for (name, _) in OPERATIONS {
        let verb = verb_of(name);
        match operation_ui::spec(name) {
            Some(ui) => {
                assert_eq!(ui.verb, verb, "{name}'s verb");
                assert!(
                    verbs.contains(verb),
                    "{name} is built and {verb} is no verb"
                );
                assert!(operation_ui::by_verb(verb).is_some(), "{verb} finds {name}");
            }
            None => {
                // An unbuilt operation's own long name is no command yet; the
                // three older commands stay, and start nothing of this registry.
                if verb == name {
                    assert!(!verbs.contains(name), "{name} is a verb and is not built");
                }
                assert!(operation_ui::by_verb(verb).is_none(), "{verb}");
            }
        }
    }
    let built: Vec<&str> = operation_ui::built().map(|ui| ui.long_name).collect();
    let in_order: Vec<&str> = OPERATIONS
        .iter()
        .filter(|(name, _)| operation_ui::spec(name).is_some())
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(built, in_order);
}

#[test]
fn each_built_operations_draft_names_only_what_arc_describes() {
    // `read-file` begins no draft: its command opens the sources browser, which
    // adds the file itself.
    for ui in operation_ui::built().filter(|ui| ui.starts != [Start::SourcesBrowser]) {
        let description = described(ui.long_name);
        let draft = begun(ui, &description);
        assert_eq!(draft.long_name, ui.long_name);
        for (key, _) in &draft.values {
            assert!(
                description.parameters.iter().any(|p| p.key == *key),
                "{} takes no {key}",
                ui.long_name
            );
        }
        for (role, _) in &draft.reads {
            assert!(
                description
                    .reads
                    .iter()
                    .any(|r| r.name == *role && r.kind == "table"),
                "{} reads no table as {role}",
                ui.long_name
            );
        }
        if let Some(field) = &ui.field {
            for key in field.keys {
                let parameter = description.parameters.iter().find(|p| p.key == *key);
                assert!(parameter.is_some(), "{}'s field edits {key}", ui.long_name);
                assert!(
                    !matches!(parameter.map(|p| &p.shape), Some(ValueShape::Choice { .. })),
                    "{}'s field types a word of a few",
                    ui.long_name
                );
            }
        }
    }
}

#[test]
fn each_built_operation_is_filed_under_its_stage() {
    for ui in operation_ui::built() {
        let stage = match ui.long_name {
            "read-file" => Stage::Connect,
            "join-tables" | "append-rows" => Stage::Model,
            _ => Stage::Shape,
        };
        assert_eq!(ui.stage, stage, "{}", ui.long_name);
    }
}
