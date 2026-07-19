//! The protocol command log (card 0029, doc-25 §5) — framework-free.
//!
//! The cmdlog discipline made structural: a **Data**-tier command
//! ([`CommandTier::Data`]) is recorded as `longname + dotted address + input`
//! and nothing else; a **View**-tier command is refused (it changes only what
//! you look at). The recorder's API takes a dotted [`address`](DataCommand)
//! string only — there is **no** parameter for a screen position, pointer
//! movement, or rectangle, so a coordinate cannot enter the log even by
//! accident. That is the point: a logged row is shape-identical to an
//! `op:`/`with:` step, so a session's rows compile straight into a protocol
//! fragment (`:export-protocol`). This cannot be retrofitted — the tier must be
//! present from the first commit — hence it lives beside the registry as data.
//!
//! No gpui import may enter this file (the standing framework-free rule,
//! mirroring `registry` / `scope`). The GPUI adapter hands each dispatched verb
//! its resolved dotted address; nothing here knows about pixels.

use crate::registry::CommandTier;

/// One recorded Data-tier command: a verb by longname, the dotted address it
/// acted on, and its optional typed input. Deliberately carries NO screen
/// coordinate — the type cannot represent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCommand {
    /// The verb's stable kebab-case longname (`"yank-address"`).
    pub longname: &'static str,
    /// The stable dotted address it acted on (`"asset.edgar_gleif.crosswalk_edges"`).
    /// Never a screen position — the grammar's whole discipline.
    pub address: String,
    /// The verb's typed input, when it carries one (`"bar"` for change-mark-type).
    pub input: Option<String>,
}

impl DataCommand {
    /// The JSONL line for this row — `op:`/`with:`-shaped so a session's rows
    /// compile into a protocol fragment. A tiny hand-serialiser (no serde dep on
    /// this crate): the three fields are the entire schema, so a coordinate has
    /// nowhere to appear.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        let input = match &self.input {
            Some(v) => format!(",\"input\":{}", json_string(v)),
            None => String::new(),
        };
        format!(
            "{{\"longname\":{},\"address\":{}{}}}",
            json_string(self.longname),
            json_string(&self.address),
            input
        )
    }
}

/// Minimal JSON string escaper for the four characters that must be escaped in
/// the addresses / longnames this log ever sees (`"`, `\`, newline, tab).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Why a `record` call was refused — surfaced so a mis-tiered wiring is loud,
/// never silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordRejection {
    /// A View-tier command was handed to the log. View commands never log.
    ViewTierNeverLogs,
}

/// The append-only protocol command log: Data-tier rows only, in order.
#[derive(Debug, Default, Clone)]
pub struct ProtocolCmdLog {
    rows: Vec<DataCommand>,
}

impl ProtocolCmdLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a dispatched verb. A `Data`-tier verb is appended (by longname +
    /// dotted address + input) and returned; a `View`-tier verb is refused —
    /// the tier gate that keeps navigation out of the log. There is no path by
    /// which a screen position enters: the signature has no coordinate to pass.
    ///
    /// # Errors
    /// [`RecordRejection::ViewTierNeverLogs`] when `tier` is [`CommandTier::View`].
    pub fn record(
        &mut self,
        tier: CommandTier,
        longname: &'static str,
        address: impl Into<String>,
        input: Option<String>,
    ) -> Result<&DataCommand, RecordRejection> {
        if !tier.is_logged() {
            return Err(RecordRejection::ViewTierNeverLogs);
        }
        self.rows.push(DataCommand { longname, address: address.into(), input });
        Ok(self.rows.last().expect("just pushed"))
    }

    /// The logged rows, in dispatch order.
    #[must_use]
    pub fn rows(&self) -> &[DataCommand] {
        &self.rows
    }

    /// The whole log as JSONL — one row per line, the `:export-protocol` feed.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        self.rows.iter().map(DataCommand::to_jsonl).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t29_data_command_logs_by_longname_and_dotted_address() {
        let mut log = ProtocolCmdLog::new();
        let row = log
            .record(
                CommandTier::Data,
                "yank-address",
                "asset.edgar_gleif.crosswalk_edges",
                None,
            )
            .expect("data-tier records");
        assert_eq!(row.longname, "yank-address");
        assert_eq!(row.address, "asset.edgar_gleif.crosswalk_edges");
        assert_eq!(log.rows().len(), 1);
        // The JSONL row is op:/with:-shaped and carries the dotted address, never
        // a coordinate (there is no field for one).
        let line = log.rows()[0].to_jsonl();
        assert_eq!(
            line,
            r#"{"longname":"yank-address","address":"asset.edgar_gleif.crosswalk_edges"}"#
        );
        assert!(!line.contains("x") || !line.contains("\"x\""));
        assert!(!line.contains("\"y\""), "no screen coordinate in the log");
        assert!(!line.to_lowercase().contains("pixel"));
    }

    #[test]
    fn t29_view_command_is_refused_never_logged() {
        let mut log = ProtocolCmdLog::new();
        let err = log
            .record(CommandTier::View, "protocol-consumer", "asset.p.x", None)
            .unwrap_err();
        assert_eq!(err, RecordRejection::ViewTierNeverLogs);
        assert!(log.rows().is_empty(), "a view command leaves no trace in the log");
    }

    #[test]
    fn t29_data_command_carries_typed_input_when_present() {
        let mut log = ProtocolCmdLog::new();
        log.record(CommandTier::Data, "change-mark-type", "view.dashboard.plot0", Some("bar".into()))
            .unwrap();
        assert_eq!(
            log.rows()[0].to_jsonl(),
            r#"{"longname":"change-mark-type","address":"view.dashboard.plot0","input":"bar"}"#
        );
    }

    #[test]
    fn t29_jsonl_is_one_row_per_line() {
        let mut log = ProtocolCmdLog::new();
        log.record(CommandTier::Data, "yank-address", "asset.p.a", None).unwrap();
        log.record(CommandTier::Data, "yank-address", "asset.p.b", None).unwrap();
        assert_eq!(log.to_jsonl().lines().count(), 2);
    }
}
