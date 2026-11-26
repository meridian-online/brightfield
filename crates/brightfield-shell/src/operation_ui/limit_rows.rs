//! `limit-rows` in the window: the top rows by a column, asked in a small form
//! in the inspector's Operation section.
//!
//! It starts from `shift-l` with the grid's cursor on a cell, or from "Top
//! rows by this column" on a column header's menu, and opens on ten rows,
//! largest first: `count` is `10` and `order_by` is the column as SQL spells
//! it, with `desc`. The preview is already an answer, and Enter keeps it
//! ([`Keeps::OnEnter`],
//! `limit_rows_opens_on_ten_rows_largest_first_and_keeps_on_enter`).
//!
//! The form holds the count, a number field where Up and Down change it by
//! one and Shift with them by ten; the direction, as the two words *largest
//! first* and *smallest first*, which set the direction of the order's first
//! item; and the order as text, for an order over several columns. The count
//! field takes the keyboard when the form opens and whenever nothing else
//! holds it. Each change re-reads the rows. A count arc refuses, such as
//! `ten` or `-1`, keeps the last good rows faded with arc's reason on the
//! status band. The step is named for its count (`<table>_first_<count>`).

use brightfield_workbench::chrome;
use egui::{Key, Modifiers};
use meridian_design::{semantic, spacing};

use super::scan::Scan;
use super::{
    Begin, By, Draft, FormCx, FormEvent, HeaderPart, Keeps, NameRule, Need, OperationUi, Stage,
    Start,
};

/// How many rows a limit opens on.
const DEFAULT_COUNT: &str = "10";

/// Which end of the order comes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    /// `desc`: the greatest value first.
    Largest,
    /// `asc`, or no direction: the least value first.
    Smallest,
}

impl Direction {
    const fn sign(self) -> &'static str {
        match self {
            Self::Largest => "desc",
            Self::Smallest => "asc",
        }
    }

    const fn words(self) -> &'static str {
        match self {
            Self::Largest => "largest first",
            Self::Smallest => "smallest first",
        }
    }
}

/// Where the first item of `order` ends: at its first comma at the top level,
/// so a comma in a name, a literal or a call's arguments ends nothing.
fn first_item_end(order: &str) -> usize {
    let mut scan = Scan::default();
    order
        .char_indices()
        .find(|&(_, c)| scan.top(c) && c == ',')
        .map_or(order.len(), |(at, _)| at)
}

/// The words of `item`, each with where it starts, split at whitespace at the
/// top level only, so `"most desc"` and `round(x, 1)` are one word each and a
/// `desc` inside them is not read as a direction.
fn tail_words(item: &str) -> Vec<(usize, &str)> {
    let mut scan = Scan::default();
    let mut words = Vec::new();
    let mut start = None;
    for (at, c) in item.char_indices() {
        let gap = scan.top(c) && c.is_whitespace();
        match (gap, start) {
            (true, Some(s)) => {
                words.push((s, &item[s..at]));
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push((s, &item[s..]));
    }
    words
}

/// The direction of `order`'s first item: `desc` is largest first, and `asc`
/// or none is smallest first, as SQL orders. `None` for an empty order.
fn direction(order: &str) -> Option<Direction> {
    let first = order[..first_item_end(order)].trim();
    if first.is_empty() {
        return None;
    }
    let desc = tail_words(first)
        .iter()
        .any(|(_, word)| word.eq_ignore_ascii_case("desc"));
    Some(if desc {
        Direction::Largest
    } else {
        Direction::Smallest
    })
}

/// `order` with its first item ordered `to`: its direction replaced where it
/// has one, written before a `nulls` where it has that, and added at its end
/// otherwise. The rest of the order is kept as typed. An empty order stays
/// empty, since a direction orders nothing alone.
fn with_direction(order: &str, to: Direction) -> String {
    let (first, rest) = order.split_at(first_item_end(order));
    if first.trim().is_empty() {
        return order.to_string();
    }
    let words = tail_words(first);
    let said = words
        .iter()
        .find(|(_, word)| word.eq_ignore_ascii_case("asc") || word.eq_ignore_ascii_case("desc"));
    let nulls = words
        .iter()
        .find(|(_, word)| word.eq_ignore_ascii_case("nulls"));
    let item = match (said, nulls) {
        (Some((at, word)), _) => {
            format!(
                "{}{}{}",
                &first[..*at],
                to.sign(),
                &first[at + word.len()..]
            )
        }
        (None, Some((at, _))) => format!("{}{} {}", &first[..*at], to.sign(), &first[*at..]),
        (None, None) => format!("{} {}", first.trim_end(), to.sign()),
    };
    format!("{item}{rest}")
}

/// The count `count` stepped by `by`, kept from 0. `None` for a count that is
/// not a whole number, which Up and Down leave as typed.
fn stepped(count: &str, by: i64) -> Option<String> {
    let now: i64 = count.trim().parse().ok()?;
    Some(now.saturating_add(by).max(0).to_string())
}

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let column = b
        .column
        .ok_or_else(|| "Limit rows needs a column.".to_string())?;
    let mut draft = Draft::empty("limit-rows");
    draft.on = b.table.map(str::to_string);
    draft.set("count", DEFAULT_COUNT.to_string());
    draft.set("order_by", format!("{} desc", (b.quote)(column)));
    Ok(draft)
}

/// The id of the form's count field.
fn count_id() -> egui::Id {
    egui::Id::new("limit-rows-count")
}

/// The id of the form's order field.
fn order_id() -> egui::Id {
    egui::Id::new("limit-rows-order")
}

/// What Up and Down, with or without Shift, step the count by this frame,
/// taken from the input so the field moves no caret with them. The Shift
/// presses are taken first, since egui's `consume_key` with no modifiers
/// takes a press held with Shift too.
fn count_step(ui: &egui::Ui) -> i64 {
    ui.input_mut(|i| {
        [
            (Modifiers::SHIFT, Key::ArrowUp, 10),
            (Modifiers::SHIFT, Key::ArrowDown, -10),
            (Modifiers::NONE, Key::ArrowUp, 1),
            (Modifiers::NONE, Key::ArrowDown, -1),
        ]
        .into_iter()
        .filter(|(modifiers, key, _)| i.consume_key(*modifiers, *key))
        .map(|(.., by)| by)
        .sum()
    })
}

/// The form: the count, the direction and the order.
fn form(draft: &mut Draft, ui: &mut egui::Ui, cx: &FormCx<'_>) -> FormEvent {
    let sem = semantic(cx.mode.is_dark());
    let font = crate::protocol::ui_font();
    let primary = chrome::colour(sem.text.primary);
    let secondary = chrome::colour(sem.text.secondary);
    let mut changed = false;

    // The count field holds the keyboard while nothing else does, so Up and
    // Down change the count from the moment the form opens.
    if ui.memory(|m| m.focused().is_none()) {
        ui.memory_mut(|m| m.request_focus(count_id()));
    }
    let mut count = draft.value("count").unwrap_or_default().to_string();
    if ui.memory(|m| m.has_focus(count_id())) {
        let by = count_step(ui);
        if let Some(next) = stepped(&count, by).filter(|_| by != 0) {
            count = next;
        }
    }
    ui.add_space(spacing::SPACE_2);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Keep")
                .font(font.clone())
                .color(secondary),
        );
        egui::TextEdit::singleline(&mut count)
            .id(count_id())
            .font(font.clone())
            .text_color(primary)
            .desired_width(spacing::SPACE_4 * 4.0)
            .show(ui);
        ui.label(
            egui::RichText::new("rows")
                .font(font.clone())
                .color(secondary),
        );
    });
    if draft.value("count") != Some(count.as_str()) {
        draft.set("count", count);
        changed = true;
    }

    let mut order = draft.value("order_by").unwrap_or_default().to_string();
    let now = direction(&order);
    ui.add_space(spacing::SPACE_2);
    ui.horizontal(|ui| {
        for to in [Direction::Largest, Direction::Smallest] {
            let words = egui::RichText::new(to.words()).font(font.clone());
            if chrome::selectable_row(ui, now == Some(to), words).clicked() && now != Some(to) {
                order = with_direction(&order, to);
            }
        }
    });
    ui.add_space(spacing::SPACE_2);
    ui.label(
        egui::RichText::new("Ordered by")
            .font(font.clone())
            .color(secondary),
    );
    egui::TextEdit::singleline(&mut order)
        .id(order_id())
        .font(font)
        .text_color(primary)
        .desired_width(f32::INFINITY)
        .show(ui);
    if draft.value("order_by") != Some(order.as_str()) {
        draft.set("order_by", order);
        changed = true;
    }

    if changed {
        FormEvent::Changed
    } else {
        FormEvent::None
    }
}

static UI: OperationUi = OperationUi {
    long_name: "limit-rows",
    verb: "limit-rows",
    stage: Stage::Shape,
    icon: "list-numbers",
    starts: &[Start::Cell, Start::Header(HeaderPart::Menu)],
    menu_words: "Top rows by this column",
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::Column,
        Need::ViewRecorded,
    ],
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: Some(form),
    name: NameRule {
        word: "first",
        from: By::Value("count"),
        with_by: false,
    },
};

/// `limit-rows`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::{direction, stepped, with_direction, Direction};

    #[test]
    fn an_orders_direction_is_its_first_items() {
        assert_eq!(direction("house_age desc"), Some(Direction::Largest));
        assert_eq!(direction("house_age DESC, rooms"), Some(Direction::Largest));
        assert_eq!(direction("house_age"), Some(Direction::Smallest));
        assert_eq!(direction("house_age asc"), Some(Direction::Smallest));
        assert_eq!(
            direction("rooms, house_age desc"),
            Some(Direction::Smallest)
        );
        assert_eq!(direction("x desc nulls last"), Some(Direction::Largest));
        // A name holding the word is no direction.
        assert_eq!(direction("\"most desc\""), Some(Direction::Smallest));
        assert_eq!(direction("\"a, b\" desc"), Some(Direction::Largest));
        assert_eq!(direction("  "), None);
        // A comma in a call's arguments or in a literal ends no item.
        assert_eq!(direction("round(x, 1) desc"), Some(Direction::Largest));
        assert_eq!(
            direction("concat(a, ', ', b) desc"),
            Some(Direction::Largest)
        );
        assert_eq!(direction("lower('it''s, desc')"), Some(Direction::Smallest));
    }

    #[test]
    fn a_direction_is_set_on_the_first_item_and_the_rest_is_kept() {
        let (small, large) = (Direction::Smallest, Direction::Largest);
        assert_eq!(with_direction("house_age desc", small), "house_age asc");
        assert_eq!(with_direction("house_age DESC", small), "house_age asc");
        assert_eq!(with_direction("house_age", large), "house_age desc");
        assert_eq!(
            with_direction("house_age desc, rooms", small),
            "house_age asc, rooms"
        );
        assert_eq!(
            with_direction("x nulls last, y", large),
            "x desc nulls last, y"
        );
        assert_eq!(with_direction("\"most desc\"", large), "\"most desc\" desc");
        assert_eq!(with_direction("\"a, b\" asc, c", large), "\"a, b\" desc, c");
        assert_eq!(with_direction("", large), "");
        assert_eq!(
            with_direction("coalesce(a, 0) asc, b", large),
            "coalesce(a, 0) desc, b"
        );
        assert_eq!(
            with_direction("round(median_income, 1)", large),
            "round(median_income, 1) desc"
        );
        assert_eq!(
            with_direction("concat(a, ', ', b) desc", small),
            "concat(a, ', ', b) asc"
        );
    }

    #[test]
    fn a_count_steps_from_zero_up_and_leaves_text_as_typed() {
        assert_eq!(stepped("10", 1).as_deref(), Some("11"));
        assert_eq!(stepped("10", -10).as_deref(), Some("0"));
        assert_eq!(stepped("3", -10).as_deref(), Some("0"));
        assert_eq!(stepped(" 7 ", 10).as_deref(), Some("17"));
        assert_eq!(stepped("ten", 1), None);
        assert_eq!(stepped("", 1), None);
    }
}
