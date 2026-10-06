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

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use brightfield_keys::BindingContext;
use brightfield_protocol::chart_history::HistoryKind;
use brightfield_protocol::{ChartChange, HistoryStore, VersionChange};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, ItemSpec, Slot, Subject, Verb,
};
use meridian_design::semantic;

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
/// reserved and unbound as *Log* and *Quality* are; a key for it waits on the
/// card that steps a chart back to a version.
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

/// **What changed, in the shelf's words**, led by the tile each change was made
/// on: `Map · colour: median_house_value added`, `median_income · y scale:
/// linear → log`.
///
/// `tile_of` names the tile a plot path draws, from the document the panel
/// belongs to. The tile is said once for a run of changes on it and again where
/// the next change is on another tile. A change on a mark past the first names
/// the mark, `x axis on mark 2`, so two marks' channels are not one row's
/// words.
///
/// A change outside the channels reads [`EDITED_OUTSIDE`] and its count of
/// lines, and follows the named changes: a version carrying both says both.
#[must_use]
pub fn change_words(changes: &[ChartChange], tile_of: &dyn Fn(&str) -> String) -> String {
    if changes.is_empty() {
        return NOTHING_CHANGED.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut tile: Option<&str> = None;
    for change in changes {
        let (plot, words) = match change {
            ChartChange::Channel {
                plot,
                mark,
                channel,
                before,
                after,
            } => {
                let mut word = channel_word(channel);
                if *mark > 0 {
                    word = format!("{word} on mark {}", mark + 1);
                }
                (
                    Some(plot.as_str()),
                    move_words(&word, before.as_deref(), after.as_deref()),
                )
            }
            ChartChange::PlotAttribute {
                plot,
                key,
                before,
                after,
            } => (
                Some(plot.as_str()),
                move_words(&attribute_word(key), before.as_deref(), after.as_deref()),
            ),
            ChartChange::EditedOutsideChannels { lines } => (
                None,
                format!("{EDITED_OUTSIDE} \u{b7} {}", lines_words(*lines)),
            ),
        };
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

/// A version's *what changed*: [`FIRST_KEPT`] for the oldest, and
/// [`change_words`] for any other.
#[must_use]
pub fn version_words(change: &VersionChange, tile_of: &dyn Fn(&str) -> String) -> String {
    match change {
        VersionChange::NoPredecessor => FIRST_KEPT.to_string(),
        VersionChange::Changes(changes) => change_words(changes, tile_of),
    }
}

/// A version's *kind*: `saved` for a save, and `before a write` for the text
/// the store recorded before a write replaced it.
fn kind_words(kind: HistoryKind) -> &'static str {
    match kind {
        HistoryKind::Save => "saved",
        HistoryKind::Checkpoint => "before a write",
    }
}

// ---------------------------------------------------------------------------
// The listing
// ---------------------------------------------------------------------------

/// One row of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
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
    /// What the edits not yet saved read, and the count of edits it was read at.
    unsaved: Option<(usize, String)>,
    /// Where each row drew in the last frame, the unsaved row first when it
    /// drew.
    drawn: Vec<egui::Rect>,
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

    /// The row for the edits not yet saved, as the panel drew it: `None` where
    /// none is held.
    #[must_use]
    pub fn unsaved_words(&self) -> Option<&str> {
        self.unsaved.as_ref().map(|(_, words)| words.as_str())
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
    /// `unsaved_key` counts the edits not yet saved and `unsaved` reads them
    /// as changes; it is asked only where the count has moved since it was last
    /// read, so a frame that changed nothing reads no file.
    pub fn sync(
        &mut self,
        source: Option<Source>,
        shown: bool,
        tile_of: &dyn Fn(&str) -> String,
        unsaved_key: usize,
        unsaved: impl FnOnce() -> Vec<ChartChange>,
    ) {
        let rising = shown && !self.shown_last;
        self.shown_last = shown;
        if !shown {
            return;
        }
        if rising || self.stale || self.source != source {
            self.source = source;
            self.listing = self.list(tile_of);
            self.stale = false;
        }
        if unsaved_key == 0 {
            self.unsaved = None;
        } else if self.unsaved.as_ref().map(|(key, _)| *key) != Some(unsaved_key) {
            self.unsaved = Some((unsaved_key, change_words(&unsaved(), tile_of)));
        }
    }

    /// Ask the store.
    fn list(&self, tile_of: &dyn Fn(&str) -> String) -> Listing {
        let Some(source) = &self.source else {
            return Listing::NoSource;
        };
        let list = match source.store.versions(&source.file) {
            Ok(list) => list,
            Err(e) => return Listing::Failed(e.reason().to_string()),
        };
        if list.versions.is_empty() {
            return Listing::Empty;
        }
        let rows = list
            .versions
            .iter()
            .map(|v| Row {
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
        Subject::new("Versions", ICON_VERSIONS, BindingContext::Workspace)
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let Listing::Listed { head, rows } = doc.versions().listing().clone() else {
            return;
        };
        let unsaved = doc.versions().unsaved_words().map(str::to_string);
        let sem = semantic(cx.mode.is_dark());
        let font = mono_font();
        let (primary, secondary, muted) = (
            chrome::colour(sem.text.primary),
            chrome::colour(sem.text.secondary),
            chrome::colour(sem.text.muted),
        );
        let width = ui.available_width();
        let mut drawn: Vec<egui::Rect> = Vec::with_capacity(rows.len() + 1);

        let line = |ui: &mut egui::Ui, columns: [(f32, &str, egui::Color32); 3]| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
            for (i, (x, text, colour)) in columns.iter().enumerate() {
                let right = columns.get(i + 1).map_or(width, |next| next.0) - RIGHT_PAD;
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
                    );
                    drawn.push(rect);
                    n += 1;
                }
                for row in &rows {
                    let rect = ui.cursor();
                    if n % 2 == 1 {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(rect.min, egui::vec2(width, ROW_HEIGHT)),
                            0.0,
                            stripe,
                        );
                    }
                    drawn.push(line(
                        ui,
                        [
                            (WHEN_X, &row.when, secondary),
                            (KIND_X, &row.kind, secondary),
                            (CHANGED_X, &row.changed, primary),
                        ],
                    ));
                    n += 1;
                }
            });
        doc.versions_mut().drawn = drawn;
    }
}

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
