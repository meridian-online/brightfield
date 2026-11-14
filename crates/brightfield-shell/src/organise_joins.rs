//! The organise sheet's state: the joins a Protocol's model holds, each ticked as it was decided
//! and as the analyst has since ticked it, and the keys that move between them, tick them and
//! apply the change. It draws nothing; [`crate::organise_joins_view`] draws it.
//!
//! The sheet answers the Browser context's verbs, so `j`, `k`, `Space`, `Enter` and `Esc` mean
//! here what they mean on the sources browser's canvas. A join opens ticked when it is part of
//! the model and unticked when it is a proposal waiting on the analyst, by the model pane's own
//! test ([`brightfield_ossie::is_proposal`]): a likely join discovery found is already the
//! model's, and only a possible one waits to be kept. Applying turns only what the analyst
//! changed into the model pane's own edits: a proposal ticked is kept, a join unticked is dropped,
//! and a join left as it opened is not touched, so a proposal the analyst looked past is not
//! rejected and kept out of later discovery.

use brightfield_ossie::{is_proposal, Model};

use crate::model_inspector::ModelAction;
use crate::sources_browser::{
    browser_verb, ADD, CLOSE, GO_IN_OR_ADD, MARK_OR_ACCEPT, MOVE_DOWN, MOVE_UP,
};
use crate::sources_browser_view::CanvasJoin;

/// One join on the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The relationship's name in the model.
    pub relationship: String,
    /// The table the join reads from.
    pub left_table: String,
    /// The table the join reads into.
    pub right_table: String,
    /// How the row reads: `orders.customer_id → customers.id`, every key column on each side.
    pub label: String,
    /// Whether it was part of the model, not a proposal, when the sheet opened.
    pub was_kept: bool,
    /// Whether it is ticked now.
    pub kept: bool,
}

/// What a key asks of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Nothing beyond the sheet.
    None,
    /// Perform these edits, in order, and close the sheet.
    Apply(Vec<ModelAction>),
    /// Close the sheet and change nothing.
    Close,
}

/// The organise sheet: its rows and the cursor among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Organise {
    /// The model's joins, in the model's order.
    pub rows: Vec<Row>,
    /// The row the keys act on.
    pub cursor: usize,
}

impl Organise {
    /// The sheet over `model`'s joins, or `None` when the model has fewer than two tables and so
    /// no joins to organise.
    #[must_use]
    pub fn over(model: &Model) -> Option<Self> {
        if model.datasets.len() < 2 {
            return None;
        }
        let rows = model
            .relationships
            .iter()
            .map(|r| {
                let kept = !is_proposal(r);
                Row {
                    relationship: r.name.clone(),
                    left_table: r.from.clone(),
                    right_table: r.to.clone(),
                    label: format!(
                        "{}.{} \u{2192} {}.{}",
                        r.from,
                        r.from_columns.join(", "),
                        r.to,
                        r.to_columns.join(", ")
                    ),
                    was_kept: kept,
                    kept,
                }
            })
            .collect();
        Some(Self { rows, cursor: 0 })
    }

    /// Answer the Browser-context verb `verb`. A verb the sheet does not answer does nothing.
    pub fn key(&mut self, verb: &str) -> Effect {
        match verb {
            MOVE_DOWN => {
                if self.cursor + 1 < self.rows.len() {
                    self.cursor += 1;
                }
                Effect::None
            }
            MOVE_UP => {
                self.cursor = self.cursor.saturating_sub(1);
                Effect::None
            }
            MARK_OR_ACCEPT => {
                self.toggle(self.cursor);
                Effect::None
            }
            GO_IN_OR_ADD | ADD => Effect::Apply(self.changes()),
            CLOSE => Effect::Close,
            _ => Effect::None,
        }
    }

    /// Answer a keystroke as the Browser context binds it. None of the sheet's verbs is a chord.
    pub fn press(&mut self, keystroke: &str) -> Effect {
        browser_verb(keystroke).map_or(Effect::None, |verb| self.key(verb))
    }

    /// Tick or untick the row at `at`, and put the cursor on it.
    pub fn toggle(&mut self, at: usize) {
        if let Some(row) = self.rows.get_mut(at) {
            row.kept = !row.kept;
            self.cursor = at;
        }
    }

    /// The rows as the shared join canvas draws them.
    #[must_use]
    pub fn canvas_joins(&self) -> Vec<CanvasJoin> {
        self.rows
            .iter()
            .map(|r| CanvasJoin {
                left_table: r.left_table.clone(),
                right_table: r.right_table.clone(),
                label: r.label.clone(),
                accepted: r.kept,
            })
            .collect()
    }

    /// The edits that turn the model's joins into the sheet's, in row order: Keep for each row
    /// ticked since it opened, Drop for each unticked.
    fn changes(&self) -> Vec<ModelAction> {
        self.rows
            .iter()
            .filter_map(|r| match (r.was_kept, r.kept) {
                (false, true) => Some(ModelAction::Keep(r.relationship.clone())),
                (true, false) => Some(ModelAction::Drop(r.relationship.clone())),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources_browser::GO_UP;
    use brightfield_ossie::{DecidedBy, Grade};

    const RETAIL: &str =
        include_str!("../../brightfield-ossie/fixtures/retail_extract/model.ossie.yaml");

    /// Three tables, `a`, `b` and `c`, and three possible joins: `a_to_b` the analyst kept,
    /// `c_to_b` a proposal on two columns, and `a_to_b_2`, a second proposal between `a` and `b`.
    fn model() -> Model {
        let mut m = brightfield_ossie::read(RETAIL).unwrap();
        m.datasets.truncate(3);
        for (d, name) in m.datasets.iter_mut().zip(["a", "b", "c"]) {
            d.name = name.to_string();
        }
        let template = m.relationships[0].clone();
        let join = |name: &str, from: &str, fc: &[&str], to: &str, tc: &[&str], by| {
            let mut r = template.clone();
            r.name = name.to_string();
            r.from = from.to_string();
            r.to = to.to_string();
            r.from_columns = fc.iter().map(|c| (*c).to_string()).collect();
            r.to_columns = tc.iter().map(|c| (*c).to_string()).collect();
            r.meta.grade = Grade::Possible;
            r.meta.decided_by = by;
            r
        };
        m.relationships = vec![
            join("a_to_b", "a", &["b_id"], "b", &["id"], DecidedBy::Analyst),
            join(
                "c_to_b",
                "c",
                &["b_id", "region"],
                "b",
                &["id", "region"],
                DecidedBy::Dovetail,
            ),
            join(
                "a_to_b_2",
                "a",
                &["other_b"],
                "b",
                &["id"],
                DecidedBy::Dovetail,
            ),
        ];
        m
    }

    fn sheet() -> Organise {
        Organise::over(&model()).unwrap()
    }

    fn kept(s: &Organise) -> Vec<bool> {
        s.rows.iter().map(|r| r.kept).collect()
    }

    #[test]
    fn fewer_than_two_tables_has_nothing_to_organise() {
        let mut m = model();
        m.datasets.truncate(1);
        assert!(Organise::over(&m).is_none());
    }

    #[test]
    fn rows_open_ticked_as_decided() {
        let s = sheet();
        assert_eq!(kept(&s), vec![true, false, false]);
        assert_eq!(s.rows[1].label, "c.b_id, region \u{2192} b.id, region");
    }

    #[test]
    fn a_likely_join_discovery_found_is_already_the_models() {
        let mut m = model();
        m.relationships[1].meta.grade = Grade::Likely;
        let s = Organise::over(&m).unwrap();
        assert_eq!(kept(&s), vec![true, true, false]);
    }

    #[test]
    fn two_joins_between_the_same_tables_are_two_rows() {
        let mut s = sheet();
        let canvas = s.canvas_joins();
        assert_eq!(canvas.len(), 3);
        let between_a_and_b = canvas
            .iter()
            .filter(|j| j.left_table == "a" && j.right_table == "b")
            .count();
        assert_eq!(between_a_and_b, 2);
        s.toggle(2);
        assert_eq!(kept(&s), vec![true, false, true]);
    }

    #[test]
    fn space_and_enter_keep_a_proposal() {
        let mut s = sheet();
        assert_eq!(s.key(MOVE_DOWN), Effect::None);
        assert_eq!(s.key(MARK_OR_ACCEPT), Effect::None);
        assert_eq!(
            s.key(GO_IN_OR_ADD),
            Effect::Apply(vec![ModelAction::Keep("c_to_b".into())])
        );
    }

    #[test]
    fn unticking_a_kept_join_drops_it() {
        let mut s = sheet();
        s.toggle(0);
        assert_eq!(
            s.key(ADD),
            Effect::Apply(vec![ModelAction::Drop("a_to_b".into())])
        );
    }

    #[test]
    fn toggling_twice_applies_nothing() {
        let mut s = sheet();
        s.toggle(1);
        s.toggle(1);
        assert_eq!(s.key(GO_IN_OR_ADD), Effect::Apply(vec![]));
    }

    #[test]
    fn close_is_close_and_other_verbs_do_nothing() {
        let mut s = sheet();
        assert_eq!(s.key(CLOSE), Effect::Close);
        assert_eq!(s.key(GO_UP), Effect::None);
        assert_eq!(kept(&s), vec![true, false, false]);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn press_resolves_in_the_browser_context() {
        let mut s = sheet();
        assert_eq!(s.press("space"), Effect::None);
        assert_eq!(kept(&s), vec![false, false, false]);
        assert_eq!(
            s.press("enter"),
            Effect::Apply(vec![ModelAction::Drop("a_to_b".into())])
        );
        assert_eq!(s.press("escape"), Effect::Close);
    }
}
