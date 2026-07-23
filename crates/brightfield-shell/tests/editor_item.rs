//! The spec editor against the shell contract, and its save route against
//! the file — all without a GPU.
//!
//! Everything here is either a property of the editor's *declaration* (its
//! subject, its empty state, the audit) or of its *save intelligence* (the
//! byte-for-byte write, the two-writer guard, the reseed rule) — none of it
//! needs a device. The pixels are `editor_snapshot.rs`'s job.
//!
//! The contract half runs the workbench [`audit`] over a registry holding
//! the editor beside a minimal centre pane. The editor's `ItemSpec` is the
//! same one (`editor_spec`) the chart registry appends — so this is the
//! audit that entry faces, isolated from the rest of the chart view.

use std::fs;
use std::path::PathBuf;

use brightfield_keys::BindingContext;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::editor::{
    code_surface, editor_spec, highlight_line, line_col, reload_spinner_due, syntax_inks,
    treatment, yaml_layout_job, Access, EditorPane, SaveReport, TokenKind, EDITOR,
    RELOAD_SPINNER_HONESTY_MS, SAVE_ACK_MS,
};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{
    audit, Activity, EmptyState, Handled, HideAffordance, Icon, Item, ItemCtx, ItemId,
    ItemRegistry, ItemSpec, PaneKey, Request, Subject, Tone, Verb, ViewKind,
};
use meridian_design::semantic;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A spec file inside a directory unique to one test — sibling tests in this
/// process can never race each other's temp files.
fn temp_spec(test: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("bf-editor-item-{}", std::process::id()))
        .join(test);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("spec.yaml");
    fs::write(&path, contents).unwrap();
    path
}

/// An `ItemCtx` for a headless `perform`, and the queue behind it.
fn ctx(requests: &mut Vec<Request>) -> ItemCtx<'_> {
    ItemCtx::new(
        Mode::Light,
        PaneKey::new(ViewKind::Charts, EDITOR),
        egui_tiles::TileId::from_u64(1),
        true,
        requests,
    )
}

/// A buffer with everything a canonicalising save would destroy: a comment,
/// deliberate double-spacing, a blank line, real indentation, and unicode.
/// A raw string — `\`-continuations would silently eat the leading spaces
/// this buffer exists to preserve.
const PRESERVATION_BUFFER: &str = r#"# hand-written — the comment a marshaller would eat
title:  'two spaces kept'

plots:
  - mark: dot
    label: "μ per day"
"#;

// ---------------------------------------------------------------------------
// The contract
// ---------------------------------------------------------------------------

/// The minimal centre pane the audit's registry needs (a view must have
/// exactly one `Slot::Centre`).
struct TestCentre;

const TEST_CENTRE: ItemId = ItemId::new("editor-test-centre");

impl Item<ChartDoc> for TestCentre {
    fn item_id(&self) -> ItemId {
        TEST_CENTRE
    }
    fn empty_state(&self, _doc: &ChartDoc) -> Option<EmptyState> {
        Some(EmptyState::new(
            Icon("chart"),
            "Nothing to draw",
            "A stand-in centre pane for the editor's audit.",
        ))
    }
    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Centre", Icon("chart"), BindingContext::Workspace)
    }
    fn ui(&mut self, _doc: &mut ChartDoc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {}
}

/// The editor's real `ItemSpec`, in a registry, through the real audit: it
/// reports its declared id, answers a house-style empty state over an empty
/// document, declares only registered verbs, and carries a registered
/// toggle for its centre-tab slot.
#[test]
fn the_editor_passes_the_contract_audit_beside_a_centre_pane() {
    let registry = ItemRegistry::new(
        ViewKind::Charts,
        vec![
            ItemSpec {
                id: TEST_CENTRE,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(TestCentre),
            },
            editor_spec(),
        ],
    );
    audit(&registry, &ChartDoc::empty()).expect("the editor is on the contract");
}

/// No file open and no file to open, a real empty state; a file, none.
/// Over a document with no `spec_path` behind it the emptiness is the
/// PANE's (its buffer); a document that does carry one is content even
/// before the first drawn frame opens it — that case is
/// `a_composed_spec_reaches_the_editor_through_the_document`.
#[test]
fn the_editor_is_empty_without_a_file_and_not_with_one() {
    let doc = ChartDoc::empty();
    let mut pane = EditorPane::new();
    let empty = pane
        .empty_state(&doc)
        .expect("no file open → a real empty state");
    assert_eq!(empty.headline, "No spec open");
    assert!(!empty.body.is_empty());

    let path = temp_spec("empty-state", "plot: {}\n");
    pane.open_file(&path);
    assert_eq!(pane.empty_state(&doc), None, "a seeded buffer is content");
}

/// The document is how a composed spec reaches the editor. A document
/// carrying `spec_path` is not an empty editor even before the tab is ever
/// focused — the shell must call `ui`, not draw the empty state — and the
/// first drawn frame hands the path to `open_file`, seeding the buffer from
/// the file the dashboard was composed from.
#[test]
fn a_composed_spec_reaches_the_editor_through_the_document() {
    let path = temp_spec("through-the-document", "plot: {}\n");
    let mut doc = ChartDoc::empty();
    doc.spec_path = Some(path.clone());

    let mut pane = EditorPane::new();
    assert_eq!(
        pane.empty_state(&doc),
        None,
        "a document with a file behind it is content, not emptiness"
    );

    // One drawn frame is the handoff.
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(480.0, 320.0),
        )),
        ..Default::default()
    };
    let mut requests = Vec::new();
    let _ = ctx.run_ui(raw, |ui| {
        brightfield_shell::design::apply(ui.ctx(), Mode::Light);
        let mut icx = ItemCtx::new(
            Mode::Light,
            PaneKey::new(ViewKind::Charts, EDITOR),
            egui_tiles::TileId::from_u64(1),
            true,
            &mut requests,
        );
        pane.ui(&mut doc, ui, &mut icx);
    });

    assert_eq!(
        pane.path(),
        Some(path.as_path()),
        "the pane opened the file"
    );
    assert_eq!(
        pane.buffer(),
        Some("plot: {}\n"),
        "the buffer seeds from the composing file"
    );
}

/// What the editor declares, the shell draws — and nothing else. With a
/// file open: the file names the tab, the breadcrumb walks to it, keys
/// resolve in the editor context, and the one toolbar control is Save,
/// enabled exactly when there is something to write.
#[test]
fn the_editor_declares_contract_chrome_and_draws_none_itself() {
    let doc = ChartDoc::empty();
    let mut pane = EditorPane::new();
    let path = temp_spec("chrome", "plot: {}\n");
    pane.open_file(&path);

    let subject = pane.subject(&doc);
    assert_eq!(subject.title, "spec.yaml", "the file names the tab");
    assert_eq!(subject.key_context, BindingContext::Editor);
    assert_eq!(
        subject.breadcrumb.last().map(|c| c.label.as_str()),
        Some("spec.yaml"),
        "the breadcrumb ends at the file"
    );
    assert_eq!(subject.toolbar.len(), 1);
    let save = &subject.toolbar[0];
    assert_eq!(save.id, "editor-save");
    assert_eq!(save.verb, Verb::new("save-spec"));
    assert!(!save.enabled, "a clean buffer has nothing to save");

    pane.buffer_mut().unwrap().push_str("more: 1\n");
    let subject = pane.subject(&doc);
    assert!(subject.toolbar[0].enabled, "a dirty buffer can save");
    assert!(pane.can_save());
}

// ---------------------------------------------------------------------------
// The save route
// ---------------------------------------------------------------------------

/// The core of the byte-write rule: what is saved is the buffer, verbatim —
/// comment, double space, blank line, unicode — through the `save-spec`
/// verb's `perform` route. No marshalling means no normalisation of any of
/// them.
#[test]
fn saving_writes_the_buffer_bytes_exactly() {
    let path = temp_spec("byte-write", "old: contents\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);
    *pane.buffer_mut().unwrap() = PRESERVATION_BUFFER.to_string();

    let mut requests = Vec::new();
    let mut doc = ChartDoc::empty();
    let handled = pane.perform(&mut doc, Verb::new("save-spec"), &mut ctx(&mut requests));
    assert_eq!(handled, Handled::Yes, "the editor owns save-spec");
    assert!(
        requests.contains(&Request::Repaint),
        "the ack needs the next frame painted"
    );

    assert_eq!(
        fs::read(&path).unwrap(),
        PRESERVATION_BUFFER.as_bytes(),
        "the file holds the buffer's bytes, byte for byte"
    );
}

/// A clean buffer's save touches nothing — same mtime, `Unchanged` — so an
/// idle save never wakes a file watcher.
#[test]
fn a_save_with_nothing_to_write_touches_nothing() {
    let path = temp_spec("no-op", "plot: {}\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);
    let before = fs::metadata(&path).unwrap().modified().unwrap();

    assert_eq!(pane.save_now(), SaveReport::Unchanged);
    let after = fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(before, after, "an unchanged save must not bump the mtime");
}

/// The two-writer guard, end to end: an external edit turns the first save
/// into a refusal that *warns* (nothing written), and the second
/// consecutive save forces the buffer over it. An edit between the two
/// disarms the force — the decision is re-made for the new text.
#[test]
fn an_external_conflict_arms_and_the_second_save_forces() {
    let path = temp_spec("conflict", "seeded: 1\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);
    pane.buffer_mut().unwrap().push_str("mine: 2\n");
    fs::write(&path, "external: 3\n").unwrap();

    assert_eq!(pane.save_now(), SaveReport::ConflictArmed);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "external: 3\n",
        "an armed conflict writes nothing"
    );
    let subject = pane.subject(&ChartDoc::empty());
    let warning = subject
        .status
        .iter()
        .find(|s| s.id == "editor-warning")
        .expect("the conflict warns on the status rail");
    assert_eq!(warning.tone, Tone::Warning);

    // An edit between the two saves makes them non-consecutive: the force
    // disarms and the next save warns afresh.
    pane.buffer_mut().unwrap().push_str("more: 4\n");
    pane.note_buffer_edited();
    assert_eq!(pane.save_now(), SaveReport::ConflictArmed);

    // The second *consecutive* save forces the write.
    assert_eq!(pane.save_now(), SaveReport::ForcedWrite);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        pane.buffer().unwrap(),
        "the forced save wrote the buffer"
    );
}

/// A pristine buffer follows the disk; a dirty buffer keeps the author's
/// text. The reseed rule, driven through the pane's own poll.
#[test]
fn a_pristine_buffer_follows_an_external_change_a_dirty_one_does_not() {
    let path = temp_spec("reseed", "seeded: 1\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);

    fs::write(&path, "external: 2\n").unwrap();
    pane.poll_disk_now();
    assert_eq!(
        pane.buffer().unwrap(),
        "external: 2\n",
        "a pristine buffer adopts the external change"
    );

    pane.buffer_mut().unwrap().push_str("mine: 3\n");
    fs::write(&path, "external: 4\n").unwrap();
    pane.poll_disk_now();
    assert!(
        pane.buffer().unwrap().contains("mine: 3"),
        "a dirty buffer keeps the author's edits"
    );
}

/// The speed rule made checkable: the acknowledgement is on the subject the
/// moment the save returns — the very next `describe` (and so the very next
/// painted frame) carries it, transient, in the good tone. The dirty dot
/// clears in the same breath.
#[test]
fn the_save_ack_lands_on_the_very_next_subject() {
    let doc = ChartDoc::empty();
    let path = temp_spec("ack", "plot: {}\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);
    pane.buffer_mut().unwrap().push_str("more: 1\n");

    let before = pane.subject(&doc);
    assert!(
        !before.status.iter().any(|s| s.id == "editor-saved"),
        "no ack before a save"
    );
    assert_eq!(before.dirty, brightfield_workbench::Dirty::Edited);

    assert_eq!(pane.save_now(), SaveReport::Written);

    let after = pane.subject(&doc);
    let ack = after
        .status
        .iter()
        .find(|s| s.id == "editor-saved")
        .expect("the ack is on the immediately-next subject");
    assert_eq!(ack.tone, Tone::Good);
    assert_eq!(ack.hide, HideAffordance::Transient { ms: SAVE_ACK_MS });
    assert_eq!(after.dirty, brightfield_workbench::Dirty::Clean);
}

/// The honesty line as data: under 100 ms a reload shows nothing, at and
/// past it a cue is owed — and an editor with nothing pending shows none.
#[test]
fn the_reload_indicator_respects_the_honesty_line() {
    assert!(!reload_spinner_due(0));
    assert!(!reload_spinner_due(RELOAD_SPINNER_HONESTY_MS - 1));
    assert!(reload_spinner_due(RELOAD_SPINNER_HONESTY_MS));
    assert!(reload_spinner_due(RELOAD_SPINNER_HONESTY_MS + 400));

    let path = temp_spec("honesty", "plot: {}\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);
    let subject = pane.subject(&ChartDoc::empty());
    assert!(
        !subject
            .status
            .iter()
            .any(|s| s.id == Activity::FileWatch.id()),
        "nothing pending, no spinner"
    );
}

/// A pending reload reports in the typed activity vocabulary — the entry the
/// shell's one indicator collects — not in a bespoke spelling of its own.
#[test]
fn a_pending_reload_reports_as_typed_file_watch_activity() {
    let path = temp_spec("pending", "plot: {}\n");
    let mut pane = EditorPane::new();
    pane.open_file(&path);

    // Delete the file: the poll sees the mtime move and cannot read it back,
    // which is exactly the mid-write window the indicator exists for — the
    // reload stays pending rather than resolving.
    fs::remove_file(&path).unwrap();
    pane.poll_disk_now();
    std::thread::sleep(std::time::Duration::from_millis(
        u64::try_from(RELOAD_SPINNER_HONESTY_MS).unwrap() + 20,
    ));

    let subject = pane.subject(&ChartDoc::empty());
    let entry = subject
        .status
        .iter()
        .find(|s| s.id == Activity::FileWatch.id())
        .expect("a reload pending past the honesty line reports as activity");
    assert_eq!(entry.text, Activity::FileWatch.label());
    assert_eq!(
        entry.tone,
        Tone::Neutral,
        "activity is progress, never a status ink"
    );
}

/// A file that cannot be read at open leaves an unseeded editor that
/// refuses to save — whatever is typed, it never held the file's contents.
#[test]
fn an_unseeded_editor_refuses_to_save() {
    let dir = std::env::temp_dir()
        .join(format!("bf-editor-item-{}", std::process::id()))
        .join("unseeded");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("missing.yaml");
    let _ = fs::remove_file(&path);

    let mut pane = EditorPane::new();
    pane.open_file(&path);
    pane.buffer_mut().unwrap().push_str("typed: 1\n");
    assert!(!pane.can_save(), "an unseeded editor cannot offer Save");
    assert_eq!(pane.save_now(), SaveReport::RefusedUnseeded);
    assert!(!path.exists(), "the refusal wrote nothing");
}

/// The save route marshals nothing: the editor module names no spec type
/// and no serialiser. A source assertion, because the absence of a
/// dependency is exactly the kind of property a refactor re-adds without
/// noticing.
#[test]
fn the_save_route_names_no_marshaller() {
    let src =
        fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor.rs"))
            .expect("the editor module is readable");
    for needle in ["brightfield_spec", "serde_yaml", "serde_json"] {
        assert!(
            !src.contains(needle),
            "src/editor.rs names {needle} — the save route must move bytes, \
             never a marshalled value"
        );
    }
}

// ---------------------------------------------------------------------------
// Read-only: a treatment, not a disablement
// ---------------------------------------------------------------------------

/// Read-only differs from editable by *surface*, and from disabled by
/// *ink*: the backgrounds are two distinct semantic surfaces per mode, the
/// text ink is identical between the accesses (and is not the disabled
/// ink), and the badge is the read-only surface's alone.
#[test]
fn read_only_is_a_treatment_not_a_disablement() {
    for dark in [false, true] {
        let editable = treatment(Access::Editable, dark);
        let read_only = treatment(Access::ReadOnly, dark);
        assert_ne!(
            editable.background, read_only.background,
            "the two accesses take two surfaces (dark={dark})"
        );
        assert_eq!(editable.badge, None);
        assert_eq!(read_only.badge, Some("read-only"));

        // The ink side: the syntax palette takes no Access at all — one
        // palette per mode — and its body text is the full-contrast primary
        // role, not the disabled role egui's alpha fade approximates.
        let inks = syntax_inks(dark);
        let sem = semantic(dark);
        assert_eq!(inks.plain, sem.text.primary);
        assert_ne!(inks.plain, sem.text.disabled);
        assert_ne!(inks.comment, sem.text.disabled);
    }
}

/// Interaction, headlessly: typing into a read-only surface changes
/// nothing, while the same keystrokes land in an editable one — and the
/// read-only surface still takes a cursor, which is what keeps selection
/// and the platform copy alive.
#[test]
fn a_read_only_surface_swallows_typing_but_keeps_its_cursor() {
    for (access, expect_changed) in [(Access::ReadOnly, false), (Access::Editable, true)] {
        let ctx = egui::Context::default();
        let mut text = String::from("plot: {}\n");
        let size = egui::vec2(480.0, 320.0);
        let mut cursor_seen = false;

        for frame in 0..4 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            match frame {
                1 => {
                    // Click into the first text row to focus the text edit.
                    let p = egui::pos2(120.0, 12.0);
                    raw.events.push(egui::Event::PointerMoved(p));
                    raw.events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    });
                    raw.events.push(egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                2 => {
                    raw.events.push(egui::Event::Text("typed".into()));
                }
                _ => {}
            }
            let _ = ctx.run_ui(raw, |ui| {
                brightfield_shell::design::apply(ui.ctx(), Mode::Light);
                let out = code_surface(ui, "test-surface", &mut text, access, Mode::Light);
                cursor_seen |= out.cursor.is_some();
            });
        }

        if expect_changed {
            assert!(
                text.contains("typed"),
                "an editable surface takes the keystrokes"
            );
        } else {
            assert_eq!(
                text, "plot: {}\n",
                "a read-only surface swallows the keystrokes"
            );
            assert!(
                cursor_seen,
                "read-only keeps a live cursor — selection and copy stay possible"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The highlighter
// ---------------------------------------------------------------------------

/// The kind covering a byte offset, for pinning one classification.
fn kind_at(line: &str, at: usize) -> TokenKind {
    highlight_line(line)
        .into_iter()
        .find(|(_, range)| range.contains(&at))
        .map(|(kind, _)| kind)
        .unwrap_or_else(|| panic!("no span covers byte {at} of {line:?}"))
}

/// The classifications the surface exists to make: keys before an unquoted
/// `: `, quoted strings, numbers and booleans, comments (whole-line and
/// trailing), anchors — and a URL's colon never splits a key.
#[test]
fn yaml_highlighting_classifies_the_line() {
    assert_eq!(kind_at("# a comment", 3), TokenKind::Comment);
    assert_eq!(kind_at("plot:", 2), TokenKind::Key);
    assert_eq!(kind_at("plot:", 4), TokenKind::Punct);
    assert_eq!(kind_at("  - mark: dot", 4), TokenKind::Key);
    assert_eq!(kind_at("  - mark: dot", 2), TokenKind::Punct);
    assert_eq!(kind_at("title: 'quoted'", 9), TokenKind::Str);
    assert_eq!(kind_at("width: 640 # px", 8), TokenKind::Literal);
    assert_eq!(kind_at("width: 640 # px", 12), TokenKind::Comment);
    assert_eq!(kind_at("base: &shared", 7), TokenKind::Anchor);
    assert_eq!(kind_at("flag: true", 7), TokenKind::Literal);
    // The URL's scheme colon is not a key separator: `src` is the key and
    // the address never becomes one.
    assert_eq!(kind_at("src: https://example.com", 1), TokenKind::Key);
    assert_ne!(kind_at("src: https://example.com", 7), TokenKind::Key);
}

/// Every byte of every line is covered by exactly one contiguous span — a
/// gap would render invisibly, an overlap doubly.
#[test]
fn highlight_spans_are_contiguous_and_cover_the_line() {
    for line in [
        "",
        "   ",
        "# comment",
        "plot:",
        "  - mark: dot",
        "title: 'unterminated",
        "nested: {a: 1, b: [2, 3]}",
        "text: μεσαίο — unicode",
        "block: |",
        "- ",
        "-",
        "anchors: [&a, *a, !tag]",
    ] {
        let spans = highlight_line(line);
        let mut at = 0;
        for (kind, range) in &spans {
            assert_eq!(
                range.start, at,
                "gap or overlap before {kind:?} in {line:?}"
            );
            assert!(range.end >= range.start);
            at = range.end;
        }
        assert_eq!(at, line.len(), "spans cover {line:?} to its end");
    }
}

/// The layout job covers the whole buffer and paints a comment line in the
/// comment ink — the LayoutJob is where the classification becomes paint,
/// so one end-to-end pin.
#[test]
fn the_layout_job_covers_every_byte_and_inks_comments() {
    let text = "# note\nplot: 1\n";
    let job = yaml_layout_job(text, false, egui::FontId::monospace(12.0));
    let covered: usize = job
        .sections
        .iter()
        .map(|s| s.byte_range.end.0 - s.byte_range.start.0)
        .sum();
    assert_eq!(covered, text.len(), "every byte is laid out");

    let comment_ink = brightfield_shell::design::to_color32(syntax_inks(false).comment);
    let first = &job.sections[0];
    assert_eq!(
        first.format.color, comment_ink,
        "the comment line wears the comment ink"
    );
}

/// Line/column arithmetic for the status line.
#[test]
fn line_col_is_one_based_and_newline_aware() {
    let text = "ab\ncd\n";
    assert_eq!(line_col(text, 0), (1, 1));
    assert_eq!(line_col(text, 2), (1, 3));
    assert_eq!(line_col(text, 3), (2, 1));
    assert_eq!(line_col(text, 5), (2, 3));
}
