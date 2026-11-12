//! Making Brightfield the default app for CSV, Parquet and DuckDB files, when the analyst says so.
//!
//! The first launch from the app asks once whether to, and Preferences offers the same at any
//! time. Both ask macOS through its own call for it, which shows the system's confirmation for
//! each kind of file ([`crate::mac_open::ask_default_app`]), so Brightfield does not make itself
//! the default unasked. Being one of the apps that can open the files needs no call: macOS reads
//! the document types in `packaging/Info.plist` when the app is copied into Applications and when
//! it first launches.
//!
//! The question's answer is kept in the Preferences file as [`ASKED_FIELD`], beside the Protocols
//! folder, so it is asked once. A launch that is not from the app, such as a build run from the
//! repo or a test, cannot be made the default, so it does not ask and keeps no answer, and the
//! question waits for the app's first launch.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

/// The field of the Preferences file that keeps the answer to the first launch's question, in
/// [`Answer::word`]'s words. A file without it has not been asked.
pub const ASKED_FIELD: &str = "default_app_asked";

/// One kind of file the offer covers: the name the analyst reads, and the content type macOS
/// knows it by, as the document types in `packaging/Info.plist` name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    /// The name the analyst reads, such as `CSV`.
    pub name: &'static str,
    /// The uniform type identifier macOS knows the kind by.
    pub content_type: &'static str,
}

/// The kinds of file the offer covers, in the order macOS is asked about them. The Excel, SQLite
/// and tab-separated types Brightfield also opens are left with the apps that have them.
pub const KINDS: [Kind; 3] = [
    Kind {
        name: "CSV",
        content_type: "public.comma-separated-values-text",
    },
    Kind {
        name: "Parquet",
        content_type: "org.apache.parquet",
    },
    Kind {
        name: "DuckDB",
        content_type: "org.duckdb.database",
    },
];

/// The content types of [`KINDS`], in its order, as macOS is asked about them.
#[cfg(target_os = "macos")]
const CONTENT_TYPES: [&str; 3] = [
    KINDS[0].content_type,
    KINDS[1].content_type,
    KINDS[2].content_type,
];

/// How the analyst answered the first launch's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Make Brightfield the Default: macOS was asked.
    Accepted,
    /// Not Now, Esc, or a click outside the sheet: nothing was asked.
    Declined,
}

impl Answer {
    /// The word the Preferences file keeps the answer as.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Declined => "declined",
        }
    }
}

/// Whether the Preferences file at `path` leaves the question to ask: it reads, or is missing, and
/// holds no answer. A file that is there and does not read asks nothing, since the answer could
/// not be kept in it without writing over the settings it holds.
#[must_use]
pub fn due_in(path: &Path) -> bool {
    crate::protocols_dir::read_preferences(path)
        .is_ok_and(|fields| !fields.contains_key(ASKED_FIELD))
}

/// Keep `answer` in the Preferences file at `path`, keeping each other setting it holds.
///
/// # Errors
///
/// Why the file could not be read, in which case it is left as it is, or could not be written.
pub fn record_in(path: &Path, answer: Answer) -> Result<(), String> {
    let mut fields = crate::protocols_dir::read_preferences(path).map_err(|why| {
        format!(
            "{} could not be read ({why}), so it was left as it is",
            path.display()
        )
    })?;
    fields.insert(
        ASKED_FIELD.to_owned(),
        Value::String(answer.word().to_owned()),
    );
    crate::protocols_dir::write_preferences(path, fields)
}

/// [`due_in`] for this Mac's Preferences file, in the config folder `BRIGHTFIELD_CONFIG_DIR`
/// moves; `false` where there is no config folder.
#[must_use]
pub fn due() -> bool {
    crate::protocols_dir::preferences_path().is_some_and(|path| due_in(&path))
}

/// [`record_in`] for this Mac's Preferences file.
///
/// # Errors
///
/// As [`record_in`], and a machine with no config folder.
pub fn record(answer: Answer) -> Result<(), String> {
    let path = crate::protocols_dir::preferences_path()
        .ok_or_else(|| "This Mac has no folder to keep Preferences in.".to_owned())?;
    record_in(&path, answer)
}

/// Whether this launch can ask macOS to make Brightfield the default: it runs from the app, on
/// macOS 12 or later. `Err` says why not, in a sentence.
///
/// # Errors
///
/// Why this launch cannot ask.
pub fn available() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::mac_open::default_app_available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Brightfield can make itself the default app on a Mac.".to_owned())
    }
}

/// macOS's answer for one kind: its place in [`KINDS`], and `Ok` or macOS's reason.
type KindAnswer = (usize, Result<(), String>);

/// macOS's answers for each kind, as they arrive: shared with the completion handlers, which run
/// off the window's thread.
#[derive(Debug, Clone, Default)]
pub struct Asking(Arc<Mutex<Vec<KindAnswer>>>);

impl Asking {
    /// Keep the answer for the kind at `index` in [`KINDS`].
    fn answer(&self, index: usize, answer: Result<(), String>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((index, answer));
    }

    /// What asking came to, once macOS has answered for each kind; `None` while it is still
    /// asking.
    #[must_use]
    pub fn finished(&self) -> Option<Outcome> {
        let answers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        (answers.len() >= KINDS.len()).then(|| {
            let mut answers = answers.clone();
            answers.sort_by_key(|(index, _)| *index);
            Outcome(
                answers
                    .into_iter()
                    .filter_map(|(index, answer)| KINDS.get(index).map(|k| (k.name, answer)))
                    .collect(),
            )
        })
    }
}

/// Ask macOS to make Brightfield the default app for each of [`KINDS`] in turn, each once macOS
/// has answered for the one before, so its confirmations come one at a time. `wake` is called
/// after each answer, off the window's thread, so the window reads [`Asking::finished`] on a
/// frame.
///
/// # Errors
///
/// Why this launch cannot ask, as [`available`] says it; nothing is asked then.
pub fn ask(wake: impl Fn() + Send + Sync + 'static) -> Result<Asking, String> {
    #[cfg(target_os = "macos")]
    {
        let asking = Asking::default();
        let answers = asking.clone();
        crate::mac_open::ask_default_app(&CONTENT_TYPES, move |index, answer| {
            answers.answer(index, answer);
            wake();
        })?;
        Ok(asking)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = wake;
        available().map(|()| Asking::default())
    }
}

/// What asking macOS came to: each kind's name with macOS's answer for it, in [`KINDS`]' order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome(pub Vec<(&'static str, Result<(), String>)>);

impl Outcome {
    /// Whether Brightfield is now the default for each kind it asked about.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.0.iter().all(|(_, answer)| answer.is_ok())
    }

    /// What it came to, in sentences for a banner: the kinds Brightfield now opens, then each kind
    /// macOS left with the app it had, with macOS's reason.
    #[must_use]
    pub fn said(&self) -> String {
        let made: Vec<&str> = self
            .0
            .iter()
            .filter(|(_, answer)| answer.is_ok())
            .map(|(name, _)| *name)
            .collect();
        let mut said = Vec::new();
        if !made.is_empty() {
            said.push(format!(
                "Brightfield now opens {} files when you double-click them.",
                listed(&made)
            ));
        }
        for (name, answer) in &self.0 {
            if let Err(why) = answer {
                said.push(format!(
                    "macOS left {name} files with the app they had: {}",
                    sentence(why)
                ));
            }
        }
        said.join(" ")
    }
}

/// `names` as prose: `CSV`, `CSV and Parquet`, `CSV, Parquet and DuckDB`.
fn listed(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `text` ending in a full stop, as macOS's reasons sometimes do not.
fn sentence(text: &str) -> String {
    let text = text.trim();
    if text.ends_with(['.', '!', '?']) {
        text.to_owned()
    } else {
        format!("{text}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(dir: &tempfile::TempDir, text: Option<&str>) -> std::path::PathBuf {
        let path = dir.path().join(crate::protocols_dir::PREFERENCES_FILE);
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        path
    }

    #[test]
    fn a_missing_file_leaves_the_question_to_ask_and_an_answer_keeps_it_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir, None);
        assert!(due_in(&path), "a new Mac is asked");
        record_in(&path, Answer::Declined).unwrap();
        assert!(!due_in(&path), "an answer is not asked again");
        let kept: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(kept[ASKED_FIELD], "declined");
    }

    #[test]
    fn the_answer_keeps_the_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(
            &dir,
            Some(r#"{"protocols_dir": "/somewhere/Protocols", "later_build": [1, 2]}"#),
        );
        assert!(due_in(&path));
        record_in(&path, Answer::Accepted).unwrap();
        let kept: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(kept[ASKED_FIELD], "accepted");
        assert_eq!(kept["protocols_dir"], "/somewhere/Protocols");
        assert_eq!(kept["later_build"], serde_json::json!([1, 2]));
    }

    /// A Preferences file that does not read holds a setting the analyst may want back, so the
    /// question is not asked and the file is not written over.
    #[test]
    fn an_unreadable_file_asks_nothing_and_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir, Some("{ not json"));
        assert!(!due_in(&path));
        let refused = record_in(&path, Answer::Accepted).unwrap_err();
        assert!(refused.contains("could not be read"), "{refused}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    /// A test binary is not the app, so macOS cannot be asked from it, and nothing is: this is
    /// also what keeps a test from changing the Mac's default apps.
    #[test]
    fn a_launch_not_from_the_app_cannot_ask() {
        let why = available().unwrap_err();
        assert!(why.contains("Brightfield"), "{why}");
        assert!(ask(|| {}).is_err(), "and asking is refused");
    }

    #[test]
    fn the_outcome_names_what_changed_and_what_macos_kept() {
        let all = Outcome(vec![
            ("CSV", Ok(())),
            ("Parquet", Ok(())),
            ("DuckDB", Ok(())),
        ]);
        assert!(all.complete());
        assert_eq!(
            all.said(),
            "Brightfield now opens CSV, Parquet and DuckDB files when you double-click them."
        );
        let some = Outcome(vec![
            ("CSV", Err("The user declined".to_owned())),
            ("Parquet", Ok(())),
            ("DuckDB", Ok(())),
        ]);
        assert!(!some.complete());
        assert_eq!(
            some.said(),
            "Brightfield now opens Parquet and DuckDB files when you double-click them. macOS \
             left CSV files with the app they had: The user declined."
        );
        let none = Outcome(vec![("CSV", Err("No.".to_owned()))]);
        assert_eq!(
            none.said(),
            "macOS left CSV files with the app they had: No."
        );
    }

    #[test]
    fn answers_arriving_out_of_order_read_in_the_kinds_order() {
        let asking = Asking::default();
        asking.answer(2, Ok(()));
        asking.answer(0, Ok(()));
        assert_eq!(asking.finished(), None, "one is still to come");
        asking.answer(1, Err("kept".to_owned()));
        let outcome = asking.finished().expect("each has answered");
        let names: Vec<&str> = outcome.0.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["CSV", "Parquet", "DuckDB"]);
    }
}
