//! **The ledger rail's Versions panel**: the versions the store keeps of one
//! chart file, newest first, each with when it was recorded, which kind it is
//! and what changed in it.
//!
//! The listing is [`HistoryStore::versions`]'s: this module asks it, words what
//! it answers, and draws the words. A version's change is data there
//! ([`ChartChange`]) and prose here, in the shelf's own words, so a version is
//! found by reading: `x axis: longitude → median_income`, `y scale: linear →
//! log`.
//!
//! **State lives on the chart document** ([`Versions`], held by
//! [`ChartDoc`]), not in the pane. The pane's empty state is read off the
//! document before the pane draws, and a listing the pane held would be
//! unreachable from that call.
//!
//! **When the store is read.** A listing opens every version's text and
//! compares each with the one before, so it is not done per frame. It is done
//! when the panel is shown after not being shown, when a Save has recorded a
//! version, and when the chart file or the store the window points at moves.
//!
//! **The cursor.** With the panel holding the keys a cursor moves over the
//! rows, and the chart is drawn as the version under it
//! ([`ChartDoc::move_version_cursor`]). The cursor is a version's id, not a
//! row's place, so a Save that lists a newer row above it leaves it on the
//! version it was on. Its row is drawn on the cursor's ground with the Step
//! back control at its end, which steps the chart back to that version as an
//! unsaved edit ([`ChartDoc::step_back_to_cursor`]).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use brightfield_keys::BindingContext;
use brightfield_protocol::chart_history::HistoryKind;
use brightfield_protocol::{ChartChange, HistoryStore, VersionChange};
use brightfield_spec::edit::ChartEdit;
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, ItemSpec, Slot, Subject, Verb,
};
use meridian_design::{control, semantic};

use crate::app::ChartDoc;
use crate::protocol::mono_font;

/// The panel's stable id.
pub const VERSIONS: ItemId = ItemId::new("chart-versions");

/// The icon is a name, resolved to paint when the icon set lands.
const ICON_VERSIONS: Icon = Icon("history");

/// What a row, the head line and the header row are high, in logical points.
pub const ROW_HEIGHT: f32 = 24.0;

/// Where the three columns begin, from the panel's left edge.
const WHEN_X: f32 = 12.0;
const KIND_X: f32 = 162.0;
const CHANGED_X: f32 = 312.0;

/// The right edge's margin, so a truncated row's ellipsis does not touch it.
const RIGHT_PAD: f32 = 12.0;

/// What a row says about a version that has no version before it. It is the
/// first one the store kept, which is the chart as generated only when the
/// first Save held no edit.
pub const FIRST_KEPT: &str = "the first version kept";

/// What a version that differs from the one before it in no channel and no
/// line reads.
pub const NOTHING_CHANGED: &str = "nothing changed";

/// What a change outside the channels reads before its count of lines.
pub const EDITED_OUTSIDE: &str = "edited outside brightfield";

// ---------------------------------------------------------------------------
// The registry entry
// ---------------------------------------------------------------------------

/// The panel's registry entry, appended to the chart view's registry
/// ([`crate::app::chart_registry`]).
///
/// The show verb is the registry's requirement of every pane (the item audit),
/// reserved and unbound as *Log* and *Quality* are.
#[must_use]
pub fn versions_spec() -> ItemSpec<ChartDoc> {
    ItemSpec {
        id: VERSIONS,
        slot: Slot::CentreTab,
        toggle: Some(Verb::new("open-chart-versions")),
        make: || Box::new(VersionsPane),
    }
}

// ---------------------------------------------------------------------------
// The clock
// ---------------------------------------------------------------------------

/// Where *now* and the local offset come from.
///
/// A baseline of the panel cannot hold `today 14:02` while the day moves, so a
/// window can be given a fixed clock ([`crate::window::MeridianApp::set_versions_env`]).
#[derive(Clone, Copy, Debug)]
pub enum Clock {
    /// The machine's clock and the offset its time zone has at that instant.
    System,
    /// A clock that stands still, at an offset that does not move.
    Fixed {
        /// The instant it reads.
        now: SystemTime,
        /// Seconds east of UTC.
        offset_secs: i32,
    },
}

impl Clock {
    fn now(&self) -> SystemTime {
        match self {
            Self::System => SystemTime::now(),
            Self::Fixed { now, .. } => *now,
        }
    }

    /// Seconds east of UTC at `at`.
    fn offset_at(&self, at: SystemTime) -> i64 {
        match self {
            Self::Fixed { offset_secs, .. } => i64::from(*offset_secs),
            Self::System => {
                use chrono::{Local, Offset, TimeZone};
                Local
                    .timestamp_opt(unix_secs(at), 0)
                    .single()
                    .map_or(0, |t| i64::from(t.offset().fix().local_minus_utc()))
            }
        }
    }

    /// `at` as local seconds since 1970-01-01 00:00.
    fn local(&self, at: SystemTime) -> i64 {
        unix_secs(at) + self.offset_at(at)
    }
}

/// Whole seconds from the Unix epoch; negative before it.
fn unix_secs(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The civil date of a day count from 1970-01-01: year, month 1 to 12, day of
/// month. Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, usize, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, usize::try_from(month - 1).unwrap_or(0), day)
}

/// **A version's *when*:** `today 14:02` for one recorded on the current day,
/// and the weekday, day, month and time, `Fri 26 Sep 16:30`, for an earlier
/// one. The day is the clock's local day.
#[must_use]
pub fn when_words(at: SystemTime, clock: &Clock) -> String {
    let then = clock.local(at);
    let now = clock.local(clock.now());
    let (day, second) = (then.div_euclid(86_400), then.rem_euclid(86_400));
    let (hour, minute) = (second / 3_600, second % 3_600 / 60);
    if day == now.div_euclid(86_400) {
        return format!("today {hour:02}:{minute:02}");
    }
    let weekday = WEEKDAYS[usize::try_from((day + 4).rem_euclid(7)).unwrap_or(0)];
    let (_, month, date) = civil_from_days(day);
    format!(
        "{weekday} {date} {} {hour:02}:{minute:02}",
        MONTHS[month % 12]
    )
}

/// **A version's time as the chart's pane header and the status band say it:**
/// `14:02` for one recorded on the current day, and [`when_words`]' weekday,
/// day, month and time for an earlier one. `as saved 14:02` reads as a time
/// without the word *today*, which the header has no room to spend.
#[must_use]
pub fn time_words(at: SystemTime, clock: &Clock) -> String {
    let when = when_words(at, clock);
    match when.strip_prefix("today ") {
        Some(time) => time.to_string(),
        None => when,
    }
}

/// How a key token the registry binds reads in a sentence: `Enter` and `Esc`
/// for `enter` and `escape`, and any other token as it is spelled.
#[must_use]
pub fn key_word(token: &str) -> &str {
    match token {
        "enter" => "Enter",
        "escape" => "Esc",
        other => other,
    }
}

/// The words the Step back control on the cursor's row reads.
pub const STEP_BACK: &str = "Step back";

// ---------------------------------------------------------------------------
// What changed, in words
// ---------------------------------------------------------------------------

/// The shelf's word for a mark's channel key: the file says `fill` where the
/// shelf says `colour`. A key the shelf has no word for is spelled as written.
fn channel_word(key: &str) -> String {
    match key {
        "x" => ShelfChannel::X.word().to_string(),
        "y" => ShelfChannel::Y.word().to_string(),
        "fill" | "stroke" => ShelfChannel::Colour.word().to_string(),
        other => other.to_string(),
    }
}

/// A plot attribute's key as words: `yScale` reads `y scale`.
fn attribute_word(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 2);
    for c in key.chars() {
        if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// What a plot draws for an attribute it carries no key for, where that is
/// worth saying: a positional scale with no `xScale` or `yScale` is linear, so
/// switching it to log reads `linear → log` and not `log added`.
fn attribute_default(key: &str) -> Option<&'static str> {
    matches!(key, "xScale" | "yScale").then_some("linear")
}

/// Which marks a channel change was on, as the words that follow the channel:
/// nothing for the marks from the first up (one change made to a plot's channel
/// reaches its marks, so it is said once), and the places of the marks
/// otherwise, `on mark 2`, `on marks 1 and 3`.
fn marks_words(marks: &[usize]) -> String {
    if marks.iter().copied().eq(0..marks.len()) {
        return String::new();
    }
    let places: Vec<String> = marks.iter().map(|m| (m + 1).to_string()).collect();
    match places.as_slice() {
        [] => String::new(),
        [one] => format!(" on mark {one}"),
        [init @ .., last] => format!(" on marks {} and {last}", init.join(", ")),
    }
}

/// One value's move: `a → b`, `b added` where the chart carried none before,
/// and `a removed` where it carries none after.
fn move_words(word: &str, before: Option<&str>, after: Option<&str>) -> String {
    match (before, after) {
        (Some(b), Some(a)) => format!("{word}: {b} \u{2192} {a}"),
        (None, Some(a)) => format!("{word}: {a} added"),
        (Some(b), None) => format!("{word}: {b} removed"),
        (None, None) => word.to_string(),
    }
}

/// `1 line`, `3 lines`, `0 lines`.
fn lines_words(lines: usize) -> String {
    if lines == 1 {
        "1 line".to_string()
    } else {
        format!("{lines} lines")
    }
}

/// One change as it is said: where it was made and the words for it.
enum Said<'a> {
    /// A channel's change, held with the marks it was made on until the run is
    /// read through, because the same move on a plot's marks is one change.
    Channel {
        plot: &'a str,
        channel: &'a str,
        before: Option<&'a str>,
        after: Option<&'a str>,
        marks: Vec<usize>,
    },
    /// Anything else, already in words.
    Words {
        plot: Option<&'a str>,
        words: String,
    },
}

/// **What changed, in the shelf's words**, led by the tile each change was made
/// on: `Map · colour: median_house_value added`, `median_income · y scale:
/// linear → log`.
///
/// `tile_of` names the tile a plot path draws, from the document the panel
/// belongs to. The tile is said once for a run of changes on it and again where
/// the next change is on another tile.
///
/// **One gesture is one change.** A column put on x reaches every mark of the
/// plot, and the store lists the move once per mark; the same move on several
/// marks of one plot is said once, as the shelf says it. A change on a mark
/// the others did not share names the mark, `x axis on mark 2`, so two marks'
/// channels are not one row's words. What the fold cannot see is a plot's marks
/// that did not change: marks 1 and 2 of a plot of three read as the plot's.
///
/// A positional scale with no key reads as linear, so a switch to log reads
/// `y scale: linear → log` whether or not the chart carried `linear` before.
///
/// A change outside the channels reads [`EDITED_OUTSIDE`] and its count of
/// lines, and follows the named changes: a version carrying both says both.
///
/// The changes one at a time are [`change_lines`], which this joins.
#[must_use]
pub fn change_words(changes: &[ChartChange], tile_of: &dyn Fn(&str) -> String) -> String {
    if changes.is_empty() {
        return NOTHING_CHANGED.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut tile: Option<&str> = None;
    for (plot, words) in said_changes(changes) {
        if let Some(plot) = plot {
            if tile != Some(plot) {
                parts.push(tile_of(plot));
                tile = Some(plot);
            }
        }
        parts.push(words);
    }
    parts.join(" \u{b7} ")
}

/// **Each change on a line of its own, in the shelf's words**, led by the tile
/// it was made on: `Map · x axis: longitude → median_income`. The changes are
/// the ones [`change_words`] says and in its order, folded the same way, so a
/// column put on x reads as one line whatever the marks it reached; a change
/// outside the channels is a line with no tile. What the close question lists
/// the unsaved edits by. No change is no line.
#[must_use]
pub fn change_lines(changes: &[ChartChange], tile_of: &dyn Fn(&str) -> String) -> Vec<String> {
    said_changes(changes)
        .into_iter()
        .map(|(plot, words)| match plot {
            Some(plot) => format!("{} \u{b7} {words}", tile_of(plot)),
            None => words,
        })
        .collect()
}

/// The changes as they are said, one entry per gesture: the plot each was made
/// on, where it was made on one, and its words.
fn said_changes(changes: &[ChartChange]) -> Vec<(Option<&str>, String)> {
    let mut said: Vec<Said<'_>> = Vec::new();
    for change in changes {
        match change {
            ChartChange::Channel {
                plot,
                mark,
                channel,
                before,
                after,
            } => {
                let same = said.iter_mut().find_map(|s| match s {
                    Said::Channel {
                        plot: p,
                        channel: c,
                        before: b,
                        after: a,
                        marks,
                    } if *p == plot.as_str()
                        && *c == channel.as_str()
                        && *b == before.as_deref()
                        && *a == after.as_deref() =>
                    {
                        Some(marks)
                    }
                    _ => None,
                });
                match same {
                    Some(marks) => {
                        if !marks.contains(mark) {
                            marks.push(*mark);
                        }
                    }
                    None => said.push(Said::Channel {
                        plot,
                        channel,
                        before: before.as_deref(),
                        after: after.as_deref(),
                        marks: vec![*mark],
                    }),
                }
            }
            ChartChange::PlotAttribute {
                plot,
                key,
                before,
                after,
            } => {
                let default = attribute_default(key);
                let (before, after) = (before.as_deref(), after.as_deref());
                // The default fills an absent side where the other side is a
                // different value. A key written at `linear` is not a move
                // (`a_scale_key_written_or_taken_out_at_linear_is_not_a_move`).
                let (before, after) = match (before, after, default) {
                    (None, Some(a), Some(d)) if a != d => (Some(d), Some(a)),
                    (Some(b), None, Some(d)) if b != d => (Some(b), Some(d)),
                    _ => (before, after),
                };
                said.push(Said::Words {
                    plot: Some(plot.as_str()),
                    words: move_words(&attribute_word(key), before, after),
                });
            }
            ChartChange::EditedOutsideChannels { lines } => said.push(Said::Words {
                plot: None,
                words: format!("{EDITED_OUTSIDE} \u{b7} {}", lines_words(*lines)),
            }),
        }
    }

    said.into_iter()
        .map(|one| match one {
            Said::Channel {
                plot,
                channel,
                before,
                after,
                mut marks,
            } => {
                marks.sort_unstable();
                let word = format!("{}{}", channel_word(channel), marks_words(&marks));
                (Some(plot), move_words(&word, before, after))
            }
            Said::Words { plot, words } => (plot, words),
        })
        .collect()
}

/// A version's *what changed*: [`FIRST_KEPT`] for the oldest, and
/// [`change_words`] for any other.
#[must_use]
pub fn version_words(change: &VersionChange, tile_of: &dyn Fn(&str) -> String) -> String {
    match change {
        VersionChange::NoPredecessor => FIRST_KEPT.to_string(),
        VersionChange::Changes(changes) => change_words(changes, tile_of),
    }
}

/// A version's *kind*: `saved` for a save, `before a write` for the text the
/// store recorded before a write replaced it, and [`CLOSED_UNSAVED`] for the
/// chart a window closed without saving.
fn kind_words(kind: HistoryKind) -> &'static str {
    match kind {
        HistoryKind::Save => "saved",
        HistoryKind::Checkpoint => "before a write",
        HistoryKind::Unsaved => CLOSED_UNSAVED,
    }
}

/// What the *kind* column reads for the chart a window closed without saving,
/// arcform's unsaved kind: the text Save would have written, never written to
/// the chart file.
pub const CLOSED_UNSAVED: &str = "closed unsaved";

/// **How a version drawn in place of the chart is named after `as`**, in the
/// pane header and the status band: [`CLOSED_UNSAVED`] for the chart a window
/// closed without saving, and `saved` for any other, which is a text the chart
/// file held.
#[must_use]
pub fn as_words(kind: HistoryKind) -> &'static str {
    match kind {
        HistoryKind::Unsaved => CLOSED_UNSAVED,
        HistoryKind::Save | HistoryKind::Checkpoint => "saved",
    }
}

// ---------------------------------------------------------------------------
// The listing
// ---------------------------------------------------------------------------

/// One row of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The id the store reads the version's text by.
    pub id: String,
    /// When the store recorded the version.
    pub at: SystemTime,
    /// The kind the store recorded it as.
    pub recorded: HistoryKind,
    /// The *when* column.
    pub when: String,
    /// The *kind* column.
    pub kind: String,
    /// The *what changed* column.
    pub changed: String,
}

/// Where the versions are read from: the store, and the chart file whose
/// versions they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The store the window's Saves record into.
    pub store: HistoryStore,
    /// The chart file, as the store keys its versions.
    pub file: PathBuf,
    /// The Protocol's folder, which the head line names the file relative to.
    pub dir: PathBuf,
}

/// What the panel has to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    /// A window with no store or no Protocol: nothing is recorded, because
    /// nothing can be.
    NoSource,
    /// The store answered with no version for the file: no Save has
    /// recorded one.
    Empty,
    /// The store could not be read, and why.
    Failed(String),
    /// The versions kept, newest first.
    Listed {
        /// The head line.
        head: String,
        /// The rows, newest first.
        rows: Vec<Row>,
    },
}

/// The ledger's Versions panel's state: what it lists and when it last listed.
#[derive(Debug)]
pub struct Versions {
    clock: Clock,
    home: Option<PathBuf>,
    source: Option<Source>,
    shown_last: bool,
    stale: bool,
    listing: Listing,
    /// What the change not yet saved reads, and what it was read from: the
    /// version the chart was stepped back to, by its id, and the edits held.
    unsaved: Option<(Option<String>, Vec<ChartEdit>, String)>,
    /// Where each row drew in the last frame, the unsaved row first when it
    /// drew.
    drawn: Vec<egui::Rect>,
    /// The version the cursor is on, by its id, or `None` before a key has put
    /// it on a row and after it has gone back to now.
    cursor: Option<String>,
    /// Where the Step back control on the cursor's row drew in the last frame.
    step_back_drawn: Option<egui::Rect>,
    /// Whether the Step back control was clicked in the last frame: read and
    /// cleared by the window's next frame, which steps the chart back.
    step_back_clicked: bool,
    /// Whether a press last frame landed in the panel (`Some(true)`) or
    /// elsewhere (`Some(false)`), or no press was made (`None`): read and
    /// cleared by the window's next frame, which gives the panel the keys or
    /// takes them away.
    pressed_in: Option<bool>,
}

impl Default for Versions {
    fn default() -> Self {
        Self {
            clock: Clock::System,
            home: std::env::var_os("HOME").map(PathBuf::from),
            source: None,
            shown_last: false,
            stale: false,
            listing: Listing::NoSource,
            unsaved: None,
            drawn: Vec::new(),
            cursor: None,
            step_back_drawn: None,
            step_back_clicked: false,
            pressed_in: None,
        }
    }
}

impl Versions {
    /// Give the panel a clock and a home folder of its own, in place of the
    /// machine's: what a suite does so a baseline does not move with the day
    /// or with whose machine it ran on.
    pub fn set_env(&mut self, clock: Clock, home: Option<PathBuf>) {
        self.clock = clock;
        self.home = home;
        self.stale = true;
    }

    /// A Save recorded a version, or something else the listing was read from
    /// moved: read the store again the next time the panel is shown.
    pub fn invalidate(&mut self) {
        self.stale = true;
        self.unsaved = None;
    }

    /// What the panel has to say as of the last [`Self::sync`].
    #[must_use]
    pub fn listing(&self) -> &Listing {
        &self.listing
    }

    /// The row for the change not yet saved, as the panel drew it: `None` when
    /// the chart is saved.
    #[must_use]
    pub fn unsaved_words(&self) -> Option<&str> {
        self.unsaved.as_ref().map(|(_, _, words)| words.as_str())
    }

    /// The version the cursor is on, by its id: `None` with the chart drawn as
    /// it is now.
    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// The listed row the cursor is on, and its place among the rows.
    #[must_use]
    pub fn cursor_row(&self) -> Option<(usize, &Row)> {
        let id = self.cursor.as_deref()?;
        let Listing::Listed { rows, .. } = &self.listing else {
            return None;
        };
        rows.iter().enumerate().find(|(_, row)| row.id == id)
    }

    /// **Move the cursor a row**, older for `down` and newer for not, and
    /// answer the row it moved to. With no cursor, either way puts it on the
    /// newest row. A move past the oldest or the newest row leaves it where it
    /// is and answers `None`, as does a panel with no row listed.
    pub fn move_cursor(&mut self, down: bool) -> Option<Row> {
        let Listing::Listed { rows, .. } = &self.listing else {
            return None;
        };
        let to = match self.cursor_row() {
            None => 0,
            Some((at, _)) if down => at + 1,
            Some((at, _)) => at.checked_sub(1)?,
        };
        let row = rows.get(to)?.clone();
        self.cursor = Some(row.id.clone());
        Some(row)
    }

    /// Take the cursor off the rows: the chart is drawn as it is now.
    pub fn clear_cursor(&mut self) {
        self.cursor = None;
    }

    /// The text the store recorded for the version `id` of the panel's chart
    /// file, or why it cannot be read.
    ///
    /// # Errors
    ///
    /// The reason, for a panel with no store to read and for a store that does
    /// not give the version up.
    pub fn read_version(&self, id: &str) -> Result<String, String> {
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| "no store holds this chart's versions".to_string())?;
        source
            .store
            .read_version(&source.file, id)
            .map_err(|e| e.reason().to_string())
    }

    /// `at` as the pane header and the status band say a version's time, by
    /// this panel's clock: [`time_words`].
    #[must_use]
    pub fn time_words(&self, at: SystemTime) -> String {
        time_words(at, &self.clock)
    }

    /// Where the Step back control drew in the last frame: `None` on a frame
    /// with no cursor on a row, or with the panel not drawn.
    #[must_use]
    pub fn step_back_drawn(&self) -> Option<egui::Rect> {
        self.step_back_drawn
    }

    /// Whether the Step back control was clicked since this was last asked,
    /// and forget that it was.
    pub fn take_step_back_click(&mut self) -> bool {
        std::mem::take(&mut self.step_back_clicked)
    }

    /// Where the last frame's press landed, in the panel or elsewhere, and
    /// forget it: `None` for a frame with no press, or with the panel not drawn.
    pub fn take_press(&mut self) -> Option<bool> {
        self.pressed_in.take()
    }

    /// Where each row drew in the last frame, the row for edits not yet saved
    /// first where there is one.
    #[must_use]
    pub fn drawn_rows(&self) -> &[egui::Rect] {
        &self.drawn
    }

    /// Bring the listing up to date for a frame in which the panel is, or is
    /// not, `shown`.
    ///
    /// `held` is the edits not yet saved, `stepped` the version the chart was
    /// stepped back to, and `unsaved` reads the two as changes; it is asked
    /// where they are not what was last read, so a frame that changed nothing
    /// reads no file.
    ///
    /// The reading is keyed on the edits and not on how many there are. A put
    /// on x and a put on y add the same number of edits, so a count cannot tell
    /// a reading of one from a reading of the other, and a panel that was not
    /// shown while one was taken back and the other made would return naming
    /// the edit the chart no longer holds. A frame in which the panel is not
    /// shown reads no edits, so on the next shown frame `held` is compared with
    /// what the panel last drew, whenever that was.
    pub fn sync(
        &mut self,
        source: Option<Source>,
        shown: bool,
        tile_of: &dyn Fn(&str) -> String,
        held: &[ChartEdit],
        stepped: Option<&str>,
        unsaved: impl FnOnce() -> Vec<ChartChange>,
    ) {
        let rising = shown && !self.shown_last;
        self.shown_last = shown;
        if !shown {
            self.step_back_drawn = None;
            return;
        }
        if rising || self.stale || self.source != source {
            self.source = source;
            self.listing = self.list(tile_of);
            self.stale = false;
            // A cursor on a version the store no longer lists is on no row.
            if self.cursor_row().is_none() {
                self.cursor = None;
            }
        }
        let read = self
            .unsaved
            .as_ref()
            .map(|(was, edits, _)| (was.as_deref(), edits.as_slice()));
        if held.is_empty() && stepped.is_none() {
            self.unsaved = None;
        } else if read != Some((stepped, held)) {
            self.unsaved = Some((
                stepped.map(str::to_string),
                held.to_vec(),
                change_words(&unsaved(), tile_of),
            ));
        }
    }

    /// Ask the store.
    fn list(&self, tile_of: &dyn Fn(&str) -> String) -> Listing {
        let Some(source) = &self.source else {
            return Listing::NoSource;
        };
        let list = match source.store.versions(&source.file) {
            Ok(list) => list,
            Err(e) => {
                // The chart file's folder is made by its first Save, or by a
                // close without saving, so before either the store cannot read
                // the file's versions for want of the folder. That is an empty history and not a failure. A store
                // that cannot be opened is a failure still, and says why.
                let unmade = source
                    .file
                    .parent()
                    .is_some_and(|folder| !folder.as_os_str().is_empty() && !folder.exists());
                if unmade && source.store.open().is_ok() {
                    return Listing::Empty;
                }
                return Listing::Failed(e.reason().to_string());
            }
        };
        if list.versions.is_empty() {
            return Listing::Empty;
        }
        let rows = list
            .versions
            .iter()
            .map(|v| Row {
                id: v.id.clone(),
                at: v.at,
                recorded: v.kind,
                when: when_words(v.at, &self.clock),
                kind: kind_words(v.kind).to_string(),
                changed: version_words(&v.change, tile_of),
            })
            .collect();
        let file = source
            .file
            .strip_prefix(&source.dir)
            .unwrap_or(&source.file)
            .display();
        let kept = list.kept();
        let head = format!(
            "{file} \u{b7} {kept} {} kept of {} \u{b7} the oldest is dropped first \u{b7} kept in {}, outside the data folder",
            if kept == 1 { "version" } else { "versions" },
            list.bound,
            tilde(&list.folder, self.home.as_deref()),
        );
        Listing::Listed { head, rows }
    }
}

/// `folder` with the home directory written `~`.
fn tilde(folder: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| folder.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => folder.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// The pane
// ---------------------------------------------------------------------------

/// `text` cut to fit `max` points of `font`, ending in an ellipsis where it was
/// cut.
fn fit(ui: &egui::Ui, text: &str, font: &egui::FontId, max: f32) -> String {
    let width = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cut: String = chars[..mid].iter().collect::<String>() + "\u{2026}";
        if width(&cut) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "\u{2026}"
}

/// The panel.
struct VersionsPane;

impl Item<ChartDoc> for VersionsPane {
    fn item_id(&self) -> ItemId {
        VERSIONS
    }

    /// No version recorded, a window that could not record one, and a store
    /// that cannot be read each say so, and the first two name Save.
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        match doc.versions().listing() {
            Listing::Listed { .. } => None,
            Listing::NoSource | Listing::Empty => Some(EmptyState::new(
                ICON_VERSIONS,
                "No version is recorded",
                "Save the chart and its first version is listed here.",
            )),
            Listing::Failed(why) => Some(EmptyState::new(
                ICON_VERSIONS,
                "The versions cannot be listed",
                why.clone(),
            )),
        }
    }

    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Versions", ICON_VERSIONS, BindingContext::Versions)
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // A press in the panel gives it the keys, and one elsewhere takes them
        // away: what the window reads the next frame.
        let panel = ui.max_rect();
        if ui.input(|i| i.pointer.any_pressed()) {
            doc.versions_mut().pressed_in = Some(ui.rect_contains_pointer(panel));
        }
        let Listing::Listed { head, rows } = doc.versions().listing().clone() else {
            doc.versions_mut().step_back_drawn = None;
            return;
        };
        let unsaved = doc.versions().unsaved_words().map(str::to_string);
        let cursor = doc.versions().cursor().map(str::to_string);
        let sem = semantic(cx.mode.is_dark());
        let font = mono_font();
        let (primary, secondary, muted) = (
            chrome::colour(sem.text.primary),
            chrome::colour(sem.text.secondary),
            chrome::colour(sem.text.muted),
        );
        let width = ui.available_width();
        let mut drawn: Vec<egui::Rect> = Vec::with_capacity(rows.len() + 1);
        let mut step_back: Option<(egui::Rect, bool)> = None;

        // A row's three columns, the last ending `end` points from the row's
        // left edge: the panel's width, or short of the controls on the
        // cursor's row.
        let line = |ui: &mut egui::Ui, columns: [(f32, &str, egui::Color32); 3], end: f32| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
            for (i, (x, text, colour)) in columns.iter().enumerate() {
                let right = columns.get(i + 1).map_or(end, |next| next.0) - RIGHT_PAD;
                let shown = fit(ui, text, &font, right - x);
                ui.painter().text(
                    rect.left_center() + egui::vec2(*x, 0.0),
                    egui::Align2::LEFT_CENTER,
                    shown,
                    font.clone(),
                    *colour,
                );
            }
            rect
        };

        // The head line: the file, how many versions are kept of how many the
        // store holds, which is dropped first, and where the store is.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
        let shown = fit(ui, &head, &font, width - WHEN_X - RIGHT_PAD);
        ui.painter().text(
            rect.left_center() + egui::vec2(WHEN_X, 0.0),
            egui::Align2::LEFT_CENTER,
            shown,
            font.clone(),
            muted,
        );
        let header = line(
            ui,
            [
                (WHEN_X, "when", secondary),
                (KIND_X, "kind", secondary),
                (CHANGED_X, "what changed", secondary),
            ],
            width,
        );
        ui.painter().hline(
            header.x_range(),
            header.bottom(),
            egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
        );

        egui::ScrollArea::vertical()
            .id_salt("chart-versions")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let stripe = chrome::colour(sem.rows.stripe_background);
                let mut n = 0usize;
                if let Some(words) = &unsaved {
                    let rect = line(
                        ui,
                        [
                            (WHEN_X, "now", primary),
                            (KIND_X, "unsaved", primary),
                            (CHANGED_X, words, primary),
                        ],
                        width,
                    );
                    drawn.push(rect);
                    n += 1;
                }
                for row in &rows {
                    let at = ui.cursor();
                    let whole = egui::Rect::from_min_size(at.min, egui::vec2(width, ROW_HEIGHT));
                    let on = cursor.as_deref() == Some(row.id.as_str());
                    if on {
                        // The cursor's row: the ground and the bar the
                        // Protocol panel's picked row has.
                        ui.painter().rect_filled(
                            whole,
                            0.0,
                            chrome::colour(sem.rows.cursor_background),
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                whole.left_top(),
                                egui::pos2(whole.left() + control::ROW_BAR_WIDTH, whole.bottom()),
                            ),
                            0.0,
                            chrome::colour(sem.rows.cursor_bar),
                        );
                    } else if n % 2 == 1 {
                        ui.painter().rect_filled(whole, 0.0, stripe);
                    }
                    // The controls are measured before the row's words draw,
                    // which stop short of them, and placed after, inside the
                    // row's own box, so placing them does not move the rows
                    // below.
                    let geometry = if on {
                        step_back_geometry(ui, whole, &font)
                    } else {
                        None
                    };
                    let end = geometry
                        .as_ref()
                        .map_or(width, |g| g.button.left() - whole.left());
                    drawn.push(line(
                        ui,
                        [
                            (WHEN_X, &row.when, secondary),
                            (KIND_X, &row.kind, secondary),
                            (CHANGED_X, &row.changed, primary),
                        ],
                        end,
                    ));
                    if let Some(g) = geometry {
                        step_back = Some(step_back_controls(ui, &g, &font, sem));
                    }
                    n += 1;
                }
            });
        let versions = doc.versions_mut();
        versions.drawn = drawn;
        versions.step_back_drawn = step_back.map(|(rect, _)| rect);
        if step_back.is_some_and(|(_, clicked)| clicked) {
            versions.step_back_clicked = true;
        }
        if let Some((rect, _)) = step_back {
            doc.controls
                .push(chrome::NamedControl::labelled(rect, STEP_BACK));
        }
    }
}

/// Where the Step back control and its key chip go at the end of a row.
struct StepBackGeometry {
    /// The Step back control's box.
    button: egui::Rect,
    /// The key chip's box.
    chip: egui::Rect,
    /// The key the registry binds `step-back-to-version` to, as a sentence
    /// says it.
    key: &'static str,
}

/// **Where the Step back control and the key that does the same go**, at the
/// trailing end of the cursor's row `row`: a button reading [`STEP_BACK`] and,
/// after it, a chip with the key the registry binds `step-back-to-version` to.
/// `None` where the verb has no key, so the row prints no chip naming one.
fn step_back_geometry(
    ui: &egui::Ui,
    row: egui::Rect,
    font: &egui::FontId,
) -> Option<StepBackGeometry> {
    let key = Verb::new(STEP_BACK_VERB).keys().map(key_word)?;
    let galley = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
    };
    let height = ROW_HEIGHT - 4.0;
    let top = row.center().y - height / 2.0;
    let chip_size = egui::vec2(galley(key).x + 2.0 * CONTROL_PAD, height);
    let button_size = egui::vec2(galley(STEP_BACK).x + 2.0 * CONTROL_PAD, height);
    let chip = egui::Rect::from_min_size(
        egui::pos2(row.right() - RIGHT_PAD - chip_size.x, top),
        chip_size,
    );
    let button = egui::Rect::from_min_size(
        egui::pos2(chip.left() - CONTROL_GAP - button_size.x, top),
        button_size,
    );
    Some(StepBackGeometry { button, chip, key })
}

/// Draw the Step back control and its key chip where `g` puts them, and answer
/// the control's box and whether it was clicked this frame.
fn step_back_controls(
    ui: &mut egui::Ui,
    g: &StepBackGeometry,
    font: &egui::FontId,
    sem: &meridian_design::Semantic,
) -> (egui::Rect, bool) {
    let response = ui.put(
        g.button,
        egui::Button::new(
            egui::RichText::new(STEP_BACK)
                .font(font.clone())
                .color(chrome::colour(sem.text.primary)),
        )
        .corner_radius(0.0)
        .min_size(g.button.size()),
    );
    ui.painter().rect_stroke(
        g.chip,
        0.0,
        egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        g.chip.center(),
        egui::Align2::CENTER_CENTER,
        g.key,
        font.clone(),
        chrome::colour(sem.text.secondary),
    );
    (response.rect, response.clicked())
}

/// The registry's verb for a step back, whose key the cursor's row prints.
pub const STEP_BACK_VERB: &str = "step-back-to-version";

/// The space inside the Step back control and the key chip, either side of
/// their words.
const CONTROL_PAD: f32 = 8.0;

/// The space between the Step back control and the key chip.
const CONTROL_GAP: f32 = 4.0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 2026-10-05 14:02:00 at UTC+10, as a count of seconds from the epoch.
    fn at(day: u32, hour: u64, minute: u64) -> SystemTime {
        // 2026-10-01 00:00 UTC is day 20_727 from the epoch.
        let days = 20_727 + u64::from(day - 1);
        UNIX_EPOCH + Duration::from_secs(days * 86_400 + hour * 3_600 + minute * 60)
    }

    fn clock() -> Clock {
        // Local 2026-10-05 14:05, UTC+10, so UTC 04:05.
        Clock::Fixed {
            now: at(5, 4, 5),
            offset_secs: 10 * 3_600,
        }
    }

    #[test]
    fn a_version_of_the_current_local_day_reads_today_and_the_time() {
        // UTC 04:02 on the 5th is 14:02 local, the clock's own day.
        assert_eq!(when_words(at(5, 4, 2), &clock()), "today 14:02");
    }

    #[test]
    fn the_day_is_the_local_day_not_the_utc_day() {
        // UTC 20:30 on the 4th is 06:30 on the 5th at UTC+10: today.
        assert_eq!(when_words(at(4, 20, 30), &clock()), "today 06:30");
        // UTC 13:59 on the 4th is 23:59 on the 4th local: yesterday.
        assert_eq!(when_words(at(4, 13, 59), &clock()), "Sun 4 Oct 23:59");
    }

    #[test]
    fn an_earlier_day_reads_weekday_day_month_and_time() {
        // 2026-09-26 06:30 UTC is 16:30 local, a Saturday.
        let sep26 = UNIX_EPOCH + Duration::from_secs((20_727 - 5) * 86_400 + 6 * 3_600 + 30 * 60);
        assert_eq!(when_words(sep26, &clock()), "Sat 26 Sep 16:30");
    }

    #[test]
    fn the_civil_date_of_the_epoch_and_of_a_leap_day() {
        assert_eq!(civil_from_days(0), (1970, 0, 1));
        // 2024-02-29 is day 19_782.
        assert_eq!(civil_from_days(19_782), (2024, 1, 29));
    }

    #[test]
    fn home_is_written_as_a_tilde_and_only_where_it_is_the_prefix() {
        let home = Path::new("/home/a");
        assert_eq!(
            tilde(Path::new("/home/a/.arcform/history"), Some(home)),
            "~/.arcform/history"
        );
        assert_eq!(tilde(Path::new("/home/a"), Some(home)), "~");
        assert_eq!(
            tilde(Path::new("/home/ab/.arcform"), Some(home)),
            "/home/ab/.arcform"
        );
        assert_eq!(tilde(Path::new("/x"), None), "/x");
    }

    fn tile(plot: &str) -> String {
        match plot {
            "root/hconcat[0]" => "Map".to_string(),
            other => other.to_string(),
        }
    }

    fn channel(
        plot: &str,
        mark: usize,
        key: &str,
        before: Option<&str>,
        after: Option<&str>,
    ) -> ChartChange {
        ChartChange::Channel {
            plot: plot.to_string(),
            mark,
            channel: key.to_string(),
            before: before.map(str::to_string),
            after: after.map(str::to_string),
        }
    }

    #[test]
    fn a_column_put_on_x_reads_in_the_shelfs_words_led_by_its_tile() {
        let changes = [channel(
            "root/hconcat[0]",
            0,
            "x",
            Some("longitude"),
            Some("median_income"),
        )];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} x axis: longitude \u{2192} median_income"
        );
    }

    #[test]
    fn a_scale_switch_reads_as_the_axis_and_scale() {
        let changes = [ChartChange::PlotAttribute {
            plot: "root/hconcat[0]".to_string(),
            key: "yScale".to_string(),
            before: Some("linear".to_string()),
            after: Some("log".to_string()),
        }];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} y scale: linear \u{2192} log"
        );
    }

    #[test]
    fn a_colour_added_and_one_removed_read_as_such_and_fill_is_colour() {
        let added = [channel(
            "root/hconcat[0]",
            0,
            "fill",
            None,
            Some("median_house_value"),
        )];
        assert_eq!(
            change_words(&added, &tile),
            "Map \u{b7} colour: median_house_value added"
        );
        let removed = [channel("root/hconcat[0]", 0, "fill", Some("a"), None)];
        assert_eq!(
            change_words(&removed, &tile),
            "Map \u{b7} colour: a removed"
        );
    }

    #[test]
    fn a_change_on_a_later_mark_names_that_mark_by_its_place() {
        let changes = [channel("root/hconcat[0]", 1, "y", Some("a"), Some("b"))];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} y axis on mark 2: a \u{2192} b"
        );
    }

    #[test]
    fn the_same_move_on_every_mark_of_a_plot_is_one_change() {
        let changes = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 1, "x", Some("a"), Some("b")),
        ];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} x axis: a \u{2192} b"
        );
    }

    #[test]
    fn a_fold_sits_where_its_first_change_was_and_leaves_the_others_in_order() {
        let changes = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 0, "y", Some("c"), Some("d")),
            channel("root/hconcat[0]", 1, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 1, "y", Some("c"), Some("d")),
        ];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} x axis: a \u{2192} b \u{b7} y axis: c \u{2192} d"
        );
    }

    #[test]
    fn moves_that_differ_between_marks_stay_two_changes_and_name_the_later_mark() {
        let differ_in_the_value = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 1, "x", Some("a"), Some("c")),
        ];
        assert_eq!(
            change_words(&differ_in_the_value, &tile),
            "Map \u{b7} x axis: a \u{2192} b \u{b7} x axis on mark 2: a \u{2192} c"
        );
        let differ_in_the_channel = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 1, "y", Some("a"), Some("b")),
        ];
        assert_eq!(
            change_words(&differ_in_the_channel, &tile),
            "Map \u{b7} x axis: a \u{2192} b \u{b7} y axis on mark 2: a \u{2192} b"
        );
    }

    #[test]
    fn the_same_move_on_two_plots_is_not_folded() {
        let changes = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[1]", 0, "x", Some("a"), Some("b")),
        ];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} x axis: a \u{2192} b \u{b7} root/hconcat[1] \u{b7} x axis: a \u{2192} b"
        );
    }

    #[test]
    fn a_fold_over_marks_that_skip_the_first_names_them() {
        let from_the_second = [
            channel("root/hconcat[0]", 1, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 2, "x", Some("a"), Some("b")),
        ];
        assert_eq!(
            change_words(&from_the_second, &tile),
            "Map \u{b7} x axis on marks 2 and 3: a \u{2192} b"
        );
        let with_a_gap = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 2, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 3, "x", Some("a"), Some("b")),
        ];
        assert_eq!(
            change_words(&with_a_gap, &tile),
            "Map \u{b7} x axis on marks 1, 3 and 4: a \u{2192} b"
        );
    }

    fn attribute(key: &str, before: Option<&str>, after: Option<&str>) -> ChartChange {
        ChartChange::PlotAttribute {
            plot: "root/hconcat[0]".to_string(),
            key: key.to_string(),
            before: before.map(str::to_string),
            after: after.map(str::to_string),
        }
    }

    #[test]
    fn a_positional_scale_with_no_key_reads_as_linear() {
        assert_eq!(
            change_words(&[attribute("yScale", None, Some("log"))], &tile),
            "Map \u{b7} y scale: linear \u{2192} log"
        );
        assert_eq!(
            change_words(&[attribute("xScale", None, Some("sqrt"))], &tile),
            "Map \u{b7} x scale: linear \u{2192} sqrt"
        );
        assert_eq!(
            change_words(&[attribute("yScale", Some("log"), None)], &tile),
            "Map \u{b7} y scale: log \u{2192} linear"
        );
    }

    #[test]
    fn a_scale_key_written_or_taken_out_at_linear_is_not_a_move() {
        assert_eq!(
            change_words(&[attribute("yScale", None, Some("linear"))], &tile),
            "Map \u{b7} y scale: linear added"
        );
        assert_eq!(
            change_words(&[attribute("yScale", Some("linear"), None)], &tile),
            "Map \u{b7} y scale: linear removed"
        );
    }

    #[test]
    fn an_attribute_with_no_default_worth_saying_reads_added_and_removed() {
        assert_eq!(
            change_words(&[attribute("yGrid", None, Some("true"))], &tile),
            "Map \u{b7} y grid: true added"
        );
        assert_eq!(
            change_words(
                &[attribute("projectionType", Some("mercator"), None)],
                &tile
            ),
            "Map \u{b7} projection type: mercator removed"
        );
    }

    #[test]
    fn the_tile_is_said_once_for_a_run_and_again_where_the_tile_changes() {
        let changes = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            channel("root/hconcat[0]", 0, "y", Some("c"), Some("d")),
            channel("root/hconcat[1]", 0, "x", Some("e"), Some("f")),
        ];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} x axis: a \u{2192} b \u{b7} y axis: c \u{2192} d \u{b7} \
             root/hconcat[1] \u{b7} x axis: e \u{2192} f"
        );
    }

    #[test]
    fn an_edit_outside_the_channels_reads_its_lines_and_follows_the_named_change() {
        let changes = [
            channel("root/hconcat[0]", 0, "x", Some("a"), Some("b")),
            ChartChange::EditedOutsideChannels { lines: 3 },
        ];
        assert_eq!(
            change_words(&changes, &tile),
            "Map \u{b7} x axis: a \u{2192} b \u{b7} edited outside brightfield \u{b7} 3 lines"
        );
        assert_eq!(
            change_words(&[ChartChange::EditedOutsideChannels { lines: 1 }], &tile),
            "edited outside brightfield \u{b7} 1 line"
        );
        assert_eq!(
            change_words(&[ChartChange::EditedOutsideChannels { lines: 0 }], &tile),
            "edited outside brightfield \u{b7} 0 lines"
        );
    }

    #[test]
    fn the_oldest_version_and_a_version_that_changed_nothing_read_as_what_they_are() {
        assert_eq!(
            version_words(&VersionChange::NoPredecessor, &tile),
            FIRST_KEPT
        );
        assert_eq!(
            version_words(&VersionChange::Changes(Vec::new()), &tile),
            NOTHING_CHANGED
        );
    }
}
