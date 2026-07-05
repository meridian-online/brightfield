//! Sidebar source/column derivation (card 0017, aws_ac06) — framework-free.
//!
//! (spec AST + already-available schema) → one [`SourceListing`] per
//! `data:` source, in declaration order. NO new DuckDB round-trips: an
//! inline source reads its row keys straight from the AST; a file/query/
//! table source reads the column names of the record batches the pipeline
//! ALREADY executed for the marks that consume it (v1 approximation — the
//! executed projection, not the full table schema; DuckDB column profiling
//! is the card's next increment). No gpui import may enter this file
//! (semantic-layer rule).

use brightfield_spec::ast::{DataSourceKind, MarkData, Spec, SpecValue};
use brightfield_sql::collect_marks;

/// One `data:` source as the sidebar lists it: its name and the column
/// names known for it without asking DuckDB anything new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceListing {
    /// The source's declared name (the `data:` key).
    pub name: String,
    /// Known column names, first-seen order, deduplicated. Empty when
    /// nothing consumed the source (no executed schema to read).
    pub columns: Vec<String>,
}

/// Columns Brightfield's SQL layer synthesises (e.g. a raster's
/// `__bf_count`) — implementation detail, not source schema.
fn is_internal_column(name: &str) -> bool {
    name.starts_with("__bf_")
}

fn push_unique(columns: &mut Vec<String>, name: &str) {
    if !is_internal_column(name) && !columns.iter().any(|c| c == name) {
        columns.push(name.to_string());
    }
}

/// Derive the sidebar's source listings.
///
/// `mark_columns` is the already-available schema: per flat mark index (the
/// `collect_marks` order the pipeline executed in), the column names of that
/// mark's result batch — empty for marks that failed or returned nothing.
#[must_use]
pub fn derive_source_listings(spec: &Spec, mark_columns: &[Vec<String>]) -> Vec<SourceListing> {
    let marks = collect_marks(spec);
    spec.data
        .iter()
        .map(|(name, source)| {
            let mut columns: Vec<String> = Vec::new();
            // Inline rows carry their schema in the AST: the union of row
            // object keys, first-seen order.
            if let DataSourceKind::InlineRows(rows) = &source.kind {
                for row in rows {
                    if let SpecValue::Object(fields) = row {
                        for key in fields.keys() {
                            push_unique(&mut columns, key);
                        }
                    }
                }
            }
            // Every other kind reads the executed batches of the marks that
            // consume this source (also unioned in for inline sources — a
            // mark's projection can only confirm what the AST already said).
            for (i, mark) in marks.iter().enumerate() {
                let consumes = matches!(
                    &mark.data,
                    Some(MarkData::From { source, .. }) if source == name
                );
                if !consumes {
                    continue;
                }
                for column in mark_columns.get(i).map(Vec::as_slice).unwrap_or(&[]) {
                    push_unique(&mut columns, column);
                }
            }
            SourceListing { name: name.clone(), columns }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};

    /// aws_ac06: a multi-source spec derives one listing per source, in
    /// declaration order — inline columns from the AST, file-backed
    /// columns from the already-executed mark batches, internal helper
    /// columns filtered, and an unconsumed source listed with no columns.
    /// Pure data in, pure data out: no engine, no DuckDB.
    #[test]
    fn aws_ac06_multi_source_spec_derives_sources_and_columns() {
        const SRC: &str = r#"
data:
  points:
    - { x: 1, y: 2, species: a }
    - { x: 3, y: 4, extra: true }
  flights: { file: flights.parquet }
  unused: { file: nobody-reads-me.csv }
hconcat:
  - plot:
    - mark: dot
      data: { from: points }
      x: x
      y: y
  - plot:
    - mark: raster
      data: { from: flights }
      x: delay
      y: distance
"#;
        let spec = parse_spec(SRC, Format::Yaml).expect("parse").spec;

        // The already-available schema: per mark (collect_marks order), the
        // executed batch's columns. The raster projection includes the
        // synthesised __bf_count — internal, must not surface.
        let mark_columns = vec![
            vec!["x".to_string(), "y".to_string()],
            vec!["x_bin".to_string(), "y_bin".to_string(), "__bf_count".to_string()],
        ];

        let listings = derive_source_listings(&spec, &mark_columns);
        assert_eq!(
            listings.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["points", "flights", "unused"],
            "one listing per data: source, declaration order"
        );

        // Inline source: the union of row keys, first-seen order (the
        // second row's novel key appends; the executed columns add nothing
        // new).
        assert_eq!(listings[0].columns, vec!["x", "y", "species", "extra"]);

        // File source: the executed projection, internals filtered.
        assert_eq!(listings[1].columns, vec!["x_bin", "y_bin"]);

        // Unconsumed source: listed (the author declared it) but with no
        // columns — nothing executed reads it and v1 asks DuckDB nothing.
        assert!(listings[2].columns.is_empty());
    }

    /// aws_ac06 (failed mark): a mark whose execution failed has no batch —
    /// its source still lists, columns from whatever else is available.
    #[test]
    fn aws_ac06_failed_mark_leaves_source_listed_without_columns() {
        const SRC: &str = r#"
data:
  t: { file: t.csv }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let spec = parse_spec(SRC, Format::Yaml).expect("parse").spec;
        let listings = derive_source_listings(&spec, &[Vec::new()]);
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].name, "t");
        assert!(listings[0].columns.is_empty());
    }
}
