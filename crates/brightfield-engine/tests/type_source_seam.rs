//! The seam is one seam: a type source that is not FineType reaches every
//! caller unchanged.
//!
//! This is the test behind "swappable without touching any caller". It needs
//! no bundle, no extension and no model — which is the point. If the label
//! path from `TypeSource` to `ColumnProfile::semantic` had FineType baked into
//! it anywhere, a source that has never heard of FineType could not travel it,
//! and this file would not compile or would not pass.
//!
//! What it does NOT prove: that a non-SQL type source could be swapped in.
//! `TypeSource::typing_expr` hands back a SQL aggregate expression so the
//! typing can ride the profiling pass that already exists, and a source with
//! no SQL to offer cannot implement it. The stub below is still SQL — it is a
//! different ANSWER, not a different mechanism.

use std::sync::Arc;

use brightfield_engine::semantic::{OpenTypeSource, TypeSource, TypeSourceSpec};
use brightfield_engine::{Engine, LoadOptions, ProfileOutcome, SemanticType, ValueCheck};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};
use duckdb::arrow::array::Array;
use duckdb::Connection;

const FIXTURE: &str = r#"
data:
  things:
    - { name: "one", n: 1, tags: ["a"] }
    - { name: "two", n: 2, tags: ["b"] }
plot:
  - mark: dot
    data: { from: things }
"#;

/// A type source with nothing behind it but a rule: a column is what its
/// DuckDB type is, spelled backwards.
///
/// Deliberately absurd. A plausible stub could be passing because the real
/// path leaked through; nothing produces `RAHCRAV` by accident.
struct Backwards;

impl TypeSource for Backwards {
    fn name(&self) -> &str {
        "backwards"
    }

    fn typing_expr(&self, column: &str, duckdb_type: &str) -> Result<String, String> {
        if duckdb_type.contains('[') {
            return Err("this source does not read list columns".to_string());
        }
        // It still has to be a SQL aggregate over the column, because that is
        // what the seam asks for — but the ANSWER owes nothing to the values.
        let reversed: String = duckdb_type.chars().rev().collect();
        let literal = reversed.replace('\'', "''");
        let c = column.replace('"', "\"\"");
        Ok(format!(
            "any_value('{literal}') FILTER (WHERE \"{c}\" IS NOT NULL)"
        ))
    }

    fn read_and_check(
        &self,
        _conn: &Connection,
        _view: &str,
        _column: &str,
        cell: &dyn Array,
        row: usize,
    ) -> SemanticType {
        let label = cell
            .as_any()
            .downcast_ref::<duckdb::arrow::array::StringArray>()
            .filter(|a| !a.is_null(row))
            .map(|a| a.value(row).to_string());
        match label {
            None => SemanticType::Unlabelled,
            Some(label) => SemanticType::Labelled {
                label,
                confidence: 1.0,
                check: ValueCheck::NoConstraint,
            },
        }
    }
}

struct OpenBackwards;

impl OpenTypeSource for OpenBackwards {
    fn open(&self, _conn: &Connection) -> Result<Box<dyn TypeSource>, String> {
        Ok(Box::new(Backwards))
    }
}

#[test]
fn a_type_source_that_is_not_finetype_reaches_the_column_profile_unchanged() {
    let parsed = parse_spec(FIXTURE, Format::Yaml).expect("the fixture parses");
    let analysis = analyse_spec(&parsed.spec).expect("the fixture analyses");
    let options = LoadOptions {
        type_source: Some(TypeSourceSpec::Other(Arc::new(OpenBackwards))),
        ..LoadOptions::default()
    };
    let session = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &options)
        .expect("the fixture loads")
        .session;

    assert_eq!(
        session.type_source_name(),
        Some("backwards"),
        "the engine reports whatever came up, not what it expected"
    );
    assert_eq!(session.type_source_error(), None);

    let columns = match session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == "things")
        .expect("one source")
        .outcome
    {
        ProfileOutcome::Profiled { columns, .. } => columns,
        other => panic!("expected a profiled source, got {other:?}"),
    };

    let by_name = |n: &str| {
        columns
            .iter()
            .find(|c| c.name == n)
            .unwrap_or_else(|| panic!("no column {n}"))
            .clone()
    };

    let name = by_name("name");
    assert_eq!(name.type_name, "VARCHAR");
    assert_eq!(
        name.semantic,
        SemanticType::Labelled {
            label: "RAHCRAV".to_string(),
            confidence: 1.0,
            check: ValueCheck::NoConstraint,
        },
        "the stub's answer did not survive the trip to ColumnProfile"
    );

    let n = by_name("n");
    assert_eq!(
        n.semantic.label(),
        Some(n.type_name.chars().rev().collect::<String>().as_str()),
        "the label is this source's own function of the column, per column"
    );

    // A column the source declines is Unanswered, carrying ITS words — not
    // Unlabelled, and not the engine's paraphrase.
    match by_name("tags").semantic {
        SemanticType::Unanswered { reason } => {
            assert_eq!(reason, "this source does not read list columns");
        }
        other => panic!("expected the source's own refusal, got {other:?}"),
    }

    // Everything the profile said before a type source existed still holds:
    // the semantic pass is additive, not a replacement.
    assert_eq!(name.non_null, 2);
    assert_eq!(name.nulls, 0);
    assert_eq!(n.min.as_deref(), Some("1"));
    assert_eq!(n.max.as_deref(), Some("2"));
}
