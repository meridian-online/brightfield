//! Dock-layout persistence (card 0017, aws_ac03) — framework-free.
//!
//! The DockArea's serialised layout (`DockAreaState` JSON, versioned) is
//! SHELL-owned state: saved under the user config dir, never serialised into
//! or derived from the Mosaic spec. Every decision here — where the file
//! lives, whether a saved layout is usable, what the canvas persists, when a
//! save fires — is plain data and arithmetic on JSON text, so it runs
//! headlessly; `shell.rs` only executes the decisions. No gpui import may
//! enter this file (semantic-layer rule).

use std::io;
use std::path::{Path, PathBuf};

use crate::shell_model::CANVAS_PANEL_NAME;

/// Version stamped into the persisted layout. Bump when the panel set or
/// dock structure changes incompatibly; a mismatch falls back to the
/// default layout (no migration, no prompt).
pub const DOCK_STATE_VERSION: usize = 1;

/// Debounce for `LayoutChanged`-triggered saves, in milliseconds (their
/// dock example uses the same 10s window); quit flushes immediately.
pub const SAVE_DEBOUNCE_MS: u64 = 10_000;

/// The layout file's location: an explicit `BRIGHTFIELD_CONFIG_DIR`
/// override wins (tests, portable installs); otherwise the platform user
/// config dir. Pure in its inputs (env values are passed, not read) so the
/// selection is unit-testable.
#[must_use]
pub fn dock_state_path(
    env_override: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(dir) = env_override.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir).join("dock-state.json"));
    }
    if cfg!(target_os = "macos") {
        home.filter(|s| !s.is_empty()).map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support/Brightfield")
                .join("dock-state.json")
        })
    } else if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        Some(PathBuf::from(xdg).join("brightfield").join("dock-state.json"))
    } else {
        home.filter(|s| !s.is_empty())
            .map(|h| PathBuf::from(h).join(".config/brightfield").join("dock-state.json"))
    }
}

/// What boot does with the saved layout file's contents.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadDecision {
    /// The saved JSON is structurally sound at the expected version:
    /// restore it (the shell deserialises the carried value).
    Restore(serde_json::Value),
    /// Fall back to the default layout, with the human-readable reason.
    Default(&'static str),
}

/// Decide whether a saved layout is usable: missing, corrupt, or
/// version-mismatched files all fall back to the default layout — a bad
/// state file must never take the workspace down.
#[must_use]
pub fn decide_load(raw: Option<&str>, expected_version: usize) -> LoadDecision {
    let Some(raw) = raw else {
        return LoadDecision::Default("no saved layout");
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return LoadDecision::Default("saved layout is not valid JSON");
    };
    if !value.is_object() {
        return LoadDecision::Default("saved layout is not a JSON object");
    }
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v as usize == expected_version => LoadDecision::Restore(value),
        Some(_) => LoadDecision::Default("saved layout version mismatch"),
        None => LoadDecision::Default("saved layout carries no version"),
    }
}

/// Strip the canvas panel's serialised payload before writing (the
/// canvas-exclusion rule): the node keeps its place in the dock tree (the
/// structure needs a center) but carries no children and a null info
/// payload — the canvas is rebuilt fresh from live pipeline state each
/// boot, never deserialised from disk.
pub fn strip_canvas_state(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_canvas =
                map.get("panel_name").and_then(serde_json::Value::as_str) == Some(CANVAS_PANEL_NAME);
            if is_canvas {
                map.insert("children".to_string(), serde_json::Value::Array(Vec::new()));
                map.insert("info".to_string(), serde_json::json!({ "panel": null }));
            } else {
                for (_, v) in map.iter_mut() {
                    strip_canvas_state(v);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                strip_canvas_state(v);
            }
        }
        _ => {}
    }
}

/// What the shell should do at a save decision point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveAction {
    /// Write this serialised state to the layout file.
    Write(String),
    /// Nothing to do (not due yet, or the state hasn't changed).
    Nothing,
}

/// The save-trigger policy: debounced writes on layout changes, an
/// immediate flush on quit, and identical-state writes skipped. A pure
/// state machine over milliseconds-since-boot — the shell supplies the
/// clock and the serialised state, and executes the returned action.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavePolicy {
    /// The JSON last written (skip-if-unchanged comparand).
    last_written: Option<String>,
    /// When the pending debounced save is due; `None` = nothing pending.
    due_at_ms: Option<u64>,
}

impl SavePolicy {
    /// A layout change at `now_ms`: (re)schedule the debounced save. A
    /// burst of drags keeps pushing the deadline out — latest change wins.
    pub fn layout_changed(&mut self, now_ms: u64) {
        self.due_at_ms = Some(now_ms + SAVE_DEBOUNCE_MS);
    }

    /// The debounce timer polled at `now_ms` with the current serialised
    /// state: write when due AND changed; otherwise nothing.
    pub fn timer_fired(&mut self, now_ms: u64, state_json: &str) -> SaveAction {
        match self.due_at_ms {
            Some(due) if now_ms >= due => {
                self.due_at_ms = None;
                self.write_if_changed(state_json)
            }
            _ => SaveAction::Nothing,
        }
    }

    /// Quit flush: write immediately when the state differs from the last
    /// write, pending debounce or not — a drag followed by a quick quit
    /// must not lose the arrangement.
    pub fn quit(&mut self, state_json: &str) -> SaveAction {
        self.due_at_ms = None;
        self.write_if_changed(state_json)
    }

    fn write_if_changed(&mut self, state_json: &str) -> SaveAction {
        if self.last_written.as_deref() == Some(state_json) {
            return SaveAction::Nothing;
        }
        self.last_written = Some(state_json.to_string());
        SaveAction::Write(state_json.to_string())
    }
}

/// Read the saved layout file, if present and readable.
#[must_use]
pub fn read_state_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Write the layout file, creating the config directory as needed.
pub fn write_state_file(path: &Path, json: &str) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal DockAreaState-shaped layout (their serde shape: version +
    /// center PanelState tree + optional docks) with the canvas at center
    /// and the editor in the right dock.
    fn layout_json(version: usize) -> serde_json::Value {
        serde_json::json!({
            "version": version,
            "center": {
                "panel_name": CANVAS_PANEL_NAME,
                "children": [],
                "info": { "panel": { "stale": "live-state-must-not-persist" } }
            },
            "right_dock": {
                "panel": {
                    "panel_name": "BrightfieldSpecEditor",
                    "children": [],
                    "info": { "panel": null }
                },
                "placement": "right",
                "size": 380.0,
                "open": true
            }
        })
    }

    /// aws_ac03 (round-trip): a well-formed saved layout at the expected
    /// version restores exactly — serialise → decide_load → the same value.
    #[test]
    fn aws_ac03_round_trip_restores_saved_layout() {
        let value = layout_json(DOCK_STATE_VERSION);
        let raw = serde_json::to_string_pretty(&value).unwrap();
        match decide_load(Some(&raw), DOCK_STATE_VERSION) {
            LoadDecision::Restore(restored) => assert_eq!(restored, value),
            other => panic!("expected Restore, got {other:?}"),
        }
    }

    /// aws_ac03 (fallback decisions): missing file, corrupt JSON, a
    /// non-object root, a missing version, and a version mismatch each
    /// fall back to the default layout with a distinct reason.
    #[test]
    fn aws_ac03_missing_corrupt_or_mismatched_state_falls_back() {
        let cases: Vec<(Option<String>, &str)> = vec![
            (None, "no saved layout"),
            (Some("{ not json ".to_string()), "saved layout is not valid JSON"),
            (Some("[1, 2, 3]".to_string()), "saved layout is not a JSON object"),
            (
                Some(serde_json::to_string(&layout_json(DOCK_STATE_VERSION + 1)).unwrap()),
                "saved layout version mismatch",
            ),
            (
                Some(r#"{ "center": { "panel_name": "x", "children": [], "info": { "tabs": { "active_index": 0 } } } }"#.to_string()),
                "saved layout carries no version",
            ),
        ];
        for (raw, expected_reason) in cases {
            match decide_load(raw.as_deref(), DOCK_STATE_VERSION) {
                LoadDecision::Default(reason) => assert_eq!(reason, expected_reason),
                other => panic!("expected Default({expected_reason}), got {other:?}"),
            }
        }
    }

    /// aws_ac03 (canvas exclusion): stripping replaces the canvas node's
    /// payload with an empty shell — no children, null info — wherever it
    /// sits in the tree, while every other panel's state survives intact.
    #[test]
    fn aws_ac03_canvas_state_is_excluded_from_persistence() {
        let mut value = layout_json(DOCK_STATE_VERSION);
        strip_canvas_state(&mut value);

        let canvas = &value["center"];
        assert_eq!(canvas["panel_name"], CANVAS_PANEL_NAME, "the node itself remains");
        assert_eq!(canvas["children"], serde_json::json!([]));
        assert_eq!(
            canvas["info"],
            serde_json::json!({ "panel": null }),
            "no live-state payload persists for the canvas"
        );
        // The editor's dock state is untouched.
        assert_eq!(value["right_dock"]["size"], 380.0);
        assert_eq!(
            value["right_dock"]["panel"]["panel_name"],
            "BrightfieldSpecEditor"
        );

        // A canvas nested under a split (the user dragged docks around it)
        // is found recursively.
        let mut nested = serde_json::json!({
            "version": DOCK_STATE_VERSION,
            "center": {
                "panel_name": "StackPanel",
                "children": [
                    { "panel_name": CANVAS_PANEL_NAME, "children": [{"panel_name": "x"}],
                      "info": { "panel": { "stale": true } } }
                ],
                "info": { "stack": { "sizes": [], "axis": 0 } }
            }
        });
        strip_canvas_state(&mut nested);
        let canvas = &nested["center"]["children"][0];
        assert_eq!(canvas["children"], serde_json::json!([]));
        assert_eq!(canvas["info"], serde_json::json!({ "panel": null }));
    }

    /// aws_ac03 (save policy): LayoutChanged debounces — not due until the
    /// window elapses, a second change pushes the deadline out, an
    /// unchanged state skips the write, and quit flushes immediately.
    #[test]
    fn aws_ac03_save_policy_debounces_and_quit_flushes() {
        let mut policy = SavePolicy::default();

        // Nothing pending → the timer does nothing.
        assert_eq!(policy.timer_fired(0, "{a}"), SaveAction::Nothing);

        // A change schedules; polling before the window elapses is a no-op.
        policy.layout_changed(1_000);
        assert_eq!(policy.timer_fired(1_000 + SAVE_DEBOUNCE_MS - 1, "{a}"), SaveAction::Nothing);

        // A second change during the window pushes the deadline out.
        policy.layout_changed(2_000);
        assert_eq!(
            policy.timer_fired(1_000 + SAVE_DEBOUNCE_MS, "{a}"),
            SaveAction::Nothing,
            "the earlier deadline was superseded"
        );

        // Once due, the changed state writes…
        assert_eq!(
            policy.timer_fired(2_000 + SAVE_DEBOUNCE_MS, "{a}"),
            SaveAction::Write("{a}".to_string())
        );
        // …and an identical state at the next due point is skipped.
        policy.layout_changed(50_000);
        assert_eq!(policy.timer_fired(50_000 + SAVE_DEBOUNCE_MS, "{a}"), SaveAction::Nothing);

        // Quit flushes a pending change immediately (no debounce wait)…
        policy.layout_changed(100_000);
        assert_eq!(policy.quit("{b}"), SaveAction::Write("{b}".to_string()));
        // …and a clean quit writes nothing.
        assert_eq!(policy.quit("{b}"), SaveAction::Nothing);
    }

    /// aws_ac03 (path selection): the env override wins and the fallbacks
    /// choose a per-user config location; no home, no path.
    #[test]
    fn aws_ac03_state_path_prefers_override_then_config_dir() {
        let over = dock_state_path(Some("/tmp/bf-test"), None, Some("/Users/x"));
        assert_eq!(over, Some(PathBuf::from("/tmp/bf-test/dock-state.json")));

        let user = dock_state_path(None, Some("/Users/x/.config"), Some("/Users/x"))
            .expect("home yields a path");
        assert!(user.ends_with("dock-state.json"));
        assert!(
            user.to_string_lossy().contains("rightfield"),
            "lives under a Brightfield config dir: {user:?}"
        );

        assert_eq!(dock_state_path(None, None, None), None);
    }
}
