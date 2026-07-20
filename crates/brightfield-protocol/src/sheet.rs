//! The `S` steps sheet — framework-free.
//!
//! The asset-first canvas is powerful but disorienting the first time: "where
//! did my *step list* go?" The `S` sheet is the answer — the flat, run-ordered
//! list of steps as a plain navigable grid, the shape every orchestrator shows
//! and the one the asset inversion hides. It is also where the per-step
//! **status** and live **state** finally surface as columns
//! ("the S sheet answers asset-inversion disorientation").
//!
//! Pure data + a cursor: the app renders the rows into a `gpui-component` table
//! and drives the cursor from `j`/`k`; the headless tests drive the same
//! surface. No gpui, no vello.

use crate::contract_graph::ContractView;

/// One row of the steps sheet — a step flattened to its grid columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRow {
    /// Run order (the seam index), 0-based — the sheet's stable sort key.
    pub order: usize,
    /// Step name (the seam identity, also the dotted address tail).
    pub name: String,
    /// Human label — narrative label when present, else the name.
    pub label: String,
    /// Kind column: `sql` / `op` / `command` / `hook` / `?`.
    pub kind: &'static str,
    /// Detail column: the op name, or the SQL model path.
    pub detail: String,
    /// Terminal status from the authoritative `.json`: `ok` / `failed` /
    /// `skipped` / `?`.
    pub status: &'static str,
    /// The latest live state folded from the `.jsonl` stream, when running
    /// (`None` once the row is only the terminal `.json` state).
    pub live_state: Option<String>,
    /// Skip reason, when the step was skipped (`hash_clean` etc.).
    pub skip_reason: Option<String>,
    /// `true` for a `finetype_validate` gate — a shield, not a lineage node.
    pub gate: bool,
}

/// The steps sheet: run-ordered [`StepRow`]s plus a cursor for `j`/`k`.
#[derive(Debug, Clone)]
pub struct StepsSheet {
    rows: Vec<StepRow>,
    cursor: usize,
}

/// The grid's column headers, in order — the sheet's schema.
pub const COLUMNS: [&str; 6] = ["#", "step", "kind", "detail", "status", "live"];

impl StepsSheet {
    /// Build the sheet from a built [`ContractView`]: one row per step, ordered
    /// by the run order the seams record (not the alphabetical step map). The
    /// status/live columns carry the per-step run status and live state.
    #[must_use]
    pub fn from_view(view: &ContractView) -> Self {
        let mut rows: Vec<StepRow> = view
            .steps
            .values()
            .map(|s| {
                let order = view
                    .graph
                    .seams
                    .get(&s.name)
                    .map_or(usize::MAX, |seam| seam.index);
                StepRow {
                    order,
                    name: s.name.clone(),
                    label: s.label.clone(),
                    kind: step_kind_label(s.kind),
                    detail: s
                        .op
                        .clone()
                        .or_else(|| s.sql_text.as_ref().map(|_| "sql".to_string()))
                        .unwrap_or_default(),
                    status: step_state_label(s.state),
                    live_state: s.live_state.clone(),
                    skip_reason: s.skip_reason.clone(),
                    gate: s.gate,
                }
            })
            .collect();
        rows.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
        Self { rows, cursor: 0 }
    }

    /// Build the sheet over already-derived rows (the offline manifest path has
    /// no [`ContractView`]; the live panel synthesises rows from the seams). The
    /// rows are re-sorted into run order + the cursor starts at the top.
    #[must_use]
    pub fn from_rows(mut rows: Vec<StepRow>) -> Self {
        rows.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
        Self { rows, cursor: 0 }
    }

    /// The rows, in run order.
    #[must_use]
    pub fn rows(&self) -> &[StepRow] {
        &self.rows
    }

    /// The number of steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the sheet has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The cursor row index.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected row, if any.
    #[must_use]
    pub fn selected(&self) -> Option<&StepRow> {
        self.rows.get(self.cursor)
    }

    /// `j` — move the cursor to the next row. A wall at the last row (no wrap).
    /// Returns whether it moved.
    pub fn cursor_down(&mut self) -> bool {
        if self.cursor + 1 < self.rows.len() {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// `k` — move the cursor to the previous row. A wall at the first row.
    pub fn cursor_up(&mut self) -> bool {
        if self.cursor > 0 {
            self.cursor -= 1;
            true
        } else {
            false
        }
    }
}

/// Kind label for the grid's `kind` column.
fn step_kind_label(kind: crate::contract::StepKind) -> &'static str {
    use crate::contract::StepKind;
    match kind {
        StepKind::Sql => "sql",
        StepKind::Op => "op",
        StepKind::Command => "command",
        StepKind::Hook => "hook",
        StepKind::Unknown => "?",
    }
}

/// Status label for the grid's `status` column — the honesty stays plain:
/// `skipped` is never `ok`.
fn step_state_label(state: crate::contract::StepState) -> &'static str {
    use crate::contract::StepState;
    match state {
        StepState::Success => "ok",
        StepState::Failed => "failed",
        StepState::Skipped => "skipped",
        StepState::Unknown => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::parse_contract;
    use crate::contract_graph::build_contract_view;

    /// A minimal two-step contract with a mix of statuses + a live state.
    const CONTRACT: &str = r#"{
      "contract_version": "1.0",
      "run": { "run_id": "r1", "protocol": { "name": "demo" }, "outcome": "success" },
      "assets": [
        { "id": "file:build/a.csv", "name": "a.csv", "kind": "file", "produced_by": "fetch", "consumed_by": ["load"] },
        { "id": "table:loaded", "name": "loaded", "kind": "table", "produced_by": "load", "consumed_by": [] }
      ],
      "steps": [
        { "name": "fetch", "kind": "op", "op_ref": { "name": "http_fetch", "version_resolved": "1" }, "status": { "state": "success" } },
        { "name": "load", "kind": "sql", "sql": { "model_path": "models/load.sql", "sql_text": "CREATE TABLE loaded AS SELECT 1;", "statements": [] }, "status": { "state": "skipped", "skip_reason": "hash_clean" } }
      ]
    }"#;

    fn view() -> ContractView {
        build_contract_view(&parse_contract(CONTRACT.as_bytes()).unwrap())
    }

    #[test]
    fn steps_sheet_is_the_flat_run_ordered_list() {
        let sheet = StepsSheet::from_view(&view());
        assert_eq!(sheet.len(), 2);
        // Run order, not the alphabetical step map (fetch before load).
        assert_eq!(sheet.rows()[0].name, "fetch");
        assert_eq!(sheet.rows()[0].order, 0);
        assert_eq!(sheet.rows()[1].name, "load");
        assert_eq!(sheet.rows()[1].order, 1);
        // The kind + detail columns carry the step vocabulary.
        assert_eq!(sheet.rows()[0].kind, "op");
        assert_eq!(sheet.rows()[0].detail, "http_fetch");
        assert_eq!(sheet.rows()[1].kind, "sql");
    }

    #[test]
    fn steps_sheet_surfaces_per_step_status_and_skip_reason() {
        // The per-step run status finally shows: fetch ok, load skipped (never
        // shown as ok), with the typed skip reason.
        let sheet = StepsSheet::from_view(&view());
        assert_eq!(sheet.rows()[0].status, "ok");
        assert_eq!(sheet.rows()[1].status, "skipped");
        assert_eq!(sheet.rows()[1].skip_reason.as_deref(), Some("hash_clean"));
    }

    #[test]
    fn steps_sheet_surfaces_folded_live_state() {
        // A live `.jsonl` line for a running step overlays its live_state, which
        // the sheet's `live` column shows.
        let mut view = view();
        // Fold a synthetic live state onto `load`.
        if let Some(step) = view.steps.get_mut("load") {
            step.live_state = Some("running".to_string());
        }
        let sheet = StepsSheet::from_view(&view);
        let load = sheet.rows().iter().find(|r| r.name == "load").unwrap();
        assert_eq!(load.live_state.as_deref(), Some("running"));
    }

    #[test]
    fn steps_sheet_cursor_navigates_without_wrapping() {
        let mut sheet = StepsSheet::from_view(&view());
        assert_eq!(sheet.cursor(), 0);
        assert_eq!(sheet.selected().unwrap().name, "fetch");
        assert!(sheet.cursor_down());
        assert_eq!(sheet.selected().unwrap().name, "load");
        assert!(!sheet.cursor_down(), "wall at the last row");
        assert!(sheet.cursor_up());
        assert_eq!(sheet.cursor(), 0);
        assert!(!sheet.cursor_up(), "wall at the first row");
    }

    #[test]
    fn steps_sheet_columns_are_the_grid_schema() {
        assert_eq!(COLUMNS, ["#", "step", "kind", "detail", "status", "live"]);
    }
}
