//! Token discipline: this crate may not spell colour or the box model as raw
//! literals. Colour belongs to the Meridian scales, spacing to the ladder, radii
//! to the radius ladder — all reached through `meridian-egui`/`meridian-design`,
//! never invented inline. This test greps the crate's own `src/` for the four
//! ways that drift creeps back:
//!
//! - `Color32::from_rgb(` — a colour built from raw channels rather than a token
//!   (the sanctioned boundary is `to_color32`, which uses `from_rgba_unmultiplied`
//!   and is not matched by this precise, paren-anchored needle);
//! - a six-hex literal (`0xRRGGBB`) — a hand-typed colour, which also catches
//!   the gpui-era `rgb(0x…)` spelling;
//! - a bare float in `add_space(` / `Margin::` / `inner_margin(` / `Vec2::splat(`
//!   — a hand-typed gap or margin where a spacing/radius token belongs;
//! - a hand-rolled overlay — `egui::Window` / `egui::Area` / `egui::Modal`
//!   constructed in this crate. Overlay chrome comes from `meridian-egui`'s
//!   one `ModalLayer` / `NotificationLayer` / `ToastLayer`, or it drifts:
//!   the audit behind the design system found one conceptual modal card
//!   re-declared at four widths with three escape hints. The layers
//!   themselves construct their `Area`s inside `meridian-egui`, where the
//!   treatment is singular; this crate has no business doing so.
//!
//! Load-bearing since the overlay increment: the shell now hosts a modal
//! slot, banners and toasts, all through the `meridian-egui` layers, and
//! this file is what keeps a second copy from creeping in beside them.
//!
//! Each rule has an allowlist, capped at five entries, and every entry must
//! carry a justification. The lists ship empty — the cap and the mandatory
//! reason are what stop the gate from quietly absorbing new drift.

use std::fs;
use std::path::{Path, PathBuf};

/// One sanctioned exception: `(path suffix, line substring, justification)`.
type Allow = (&'static str, &'static str, &'static str);

const COLOUR_ALLOW: &[Allow] = &[];
const HEX_ALLOW: &[Allow] = &[];
const FLOAT_ALLOW: &[Allow] = &[];
const OVERLAY_ALLOW: &[Allow] = &[
    (
        "src/chart_item.rs",
        r#"egui::Area::new(egui::Id::new("chart-hover-readout"))"#,
        "The hover readout is a panel that follows the pointer, and meridian-egui \
     offers no layer for one: ModalLayer is a blocking dialog, NotificationLayer \
     a persistent banner, ToastLayer a transient message that places itself. \
     None of the three can be anchored to a pixel, which is the whole \
     requirement here. The rule's own rationale is that a conceptual overlay \
     must not be re-declared at several treatments; this is a first \
     declaration, not a second, and it takes its frame and its shadow from \
     chrome::overlay_frame and Elevation::Overlay rather than inventing either. \
     The entry is anchored to this Area's id salt, so a second hand-rolled \
     overlay in this file is still a violation \
     (a_second_hand_rolled_overlay_in_the_chart_pane_is_still_a_violation). \
     The standing fix is a pointer-anchored layer in meridian-egui, which is a \
     different repository and a different lane.",
    ),
    (
        "src/shelf.rs",
        r#"egui::Area::new(egui::Id::new("shelf-column-card"))"#,
        "The shelf's column list is a card hung from a band cell when the \
         navigator rail that would draw it is shut, and meridian-egui offers no \
         layer anchored to a rect: ModalLayer is a centred blocking dialog under \
         a scrim, NotificationLayer a banner at a corner, ToastLayer a message \
         that places itself. This is the second caller of the one want the hover \
         readout's entry names, a layer placed at a point, and it takes its \
         frame from the same two sources, chrome::overlay_frame and \
         Elevation::Overlay (shelf::floating_card_frame), rather than inventing \
         either. The entry is anchored to this Area's id salt, so a second \
         hand-rolled overlay in this file is still a violation \
         (a_second_hand_rolled_overlay_in_the_shelf_is_still_a_violation). The \
         standing fix is the same point-anchored layer in meridian-egui, which \
         is a different repository and a different lane, and when it lands both \
         entries go.",
    ),
    (
        "src/typed_card.rs",
        r#"egui::Area::new(egui::Id::new(STATEMENT_CARD_ID))"#,
        "The chart palette's statement form, the Area whose id is \
         STATEMENT_CARD_ID (bf-overlay-statement). It is drawn without a scrim so \
         the grid preview beside it stays readable and live while the statement \
         is typed, and ModalLayer always lays a scrim over the page. Its card \
         is meridian-egui's own overlay_frame, so the chrome is the one modal \
         treatment and only the scrim is left out. The entry is anchored to that \
         id, so a second hand-rolled overlay in this file is still a violation \
         (a_second_hand_rolled_overlay_beside_the_statement_form_is_still_a_violation).",
    ),
    (
        "src/window.rs",
        r#"egui::Modal::new(egui::Id::new("bf-overlay-sources-browser"))"#,
        "The sources browser's sheet. It holds three columns (places, a folder's list, \
         and a preview that can be a folder's canvas of joins), so it is sized to the \
         window, and ModalLayer sets a card's width to one of its two rungs, narrow or \
         default, with no third. The sheet is otherwise ModalLayer's treatment: the \
         same token scrim behind it, and sources_browser_view::frame, which spells \
         meridian-egui's card frame (the overlay surface, its default border, the \
         panel radius, the modal padding and the window shadow) because that frame \
         is private to the layer. The entry is anchored to this Modal's id salt, so \
         a second hand-rolled overlay in this file is still a violation \
         (a_second_hand_rolled_overlay_beside_the_browser_or_the_trust_band_is_still_a_violation). \
         The standing fix is a sheet width in meridian-egui's ModalLayer, which is a \
         different repository; when it lands, this entry goes.",
    ),
    (
        "src/window.rs",
        r#"egui::Area::new(egui::Id::new("bf-restricted-band"))"#,
        "The band over a restricted Protocol's canvas, which says where the Protocol \
         came from and carries Trust this Protocol and the folders above it that can \
         be trusted. The design gives trust no dialog, so it is not ModalLayer's \
         centred card under a scrim: it is anchored to the canvas's top, and the \
         Protocol's spine and the pictures of its shipped results stay in reach \
         beside it. NotificationLayer's banners carry a title and a body but no \
         buttons. That makes it a third caller of the want the hover \
         readout's and the shelf card's entries name, a layer placed at a rect, and \
         it takes its frame from egui's popup frame for the window's style rather \
         than inventing one. The entry is anchored to this Area's id salt \
         (a_second_hand_rolled_overlay_beside_the_browser_or_the_trust_band_is_still_a_violation). \
         The standing fix is that rect-anchored layer, or banners with actions, in \
         meridian-egui, a different repository; when it lands, this entry goes.",
    ),
];

/// Call fragments that construct a floating layer by hand. `egui::Modal` has
/// no paren anchor because `Modal::new(egui::Id…)` is exactly the call the
/// `meridian-egui` layer makes — the needle here is this *crate* naming the
/// type at all.
const OVERLAY_NEEDLES: &[&str] = &["egui::Window::new", "egui::Area::new", "egui::Modal"];

/// Call fragments after which a bare float is a hand-typed box-model value.
const FLOAT_CALLS: &[&str] = &["add_space(", "Margin::", "inner_margin(", "Vec2::splat("];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src/ is readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// The first `0x` followed by *exactly* six hex digits (a `0xRRGGBB` colour),
/// returned as the matched slice. `0xff`, `0x1234_5678` and longer runs do not
/// match — this is a colour needle, not a "no hex anywhere" rule.
fn six_hex(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_hex(bytes[j]) {
                j += 1;
            }
            if j - start == 6 {
                return Some(&line[i..j]);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// Whether `s` contains a bare float literal (`<digits>.<digits>`) not attached
/// to an identifier or path (so `SPACE_4` and `foo.bar` do not count, `6.0`
/// does). `s` is the slice starting at a call fragment, so a float before the
/// call on the same line is not considered.
fn has_bare_float(s: &str) -> bool {
    let b = s.as_bytes();
    for i in 0..b.len() {
        if b[i].is_ascii_digit() {
            // Must start a float: the char before is a boundary (not part of an
            // identifier, a path, or a preceding number).
            let boundary = i == 0
                || !matches!(b[i - 1], b'_' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z');
            if !boundary {
                continue;
            }
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
                return true;
            }
        }
    }
    false
}

fn allowed(allow: &[Allow], rel: &str, line: &str) -> bool {
    allow
        .iter()
        .any(|(suffix, needle, _why)| rel.ends_with(suffix) && line.contains(needle))
}

#[test]
fn allowlists_stay_small_and_justified() {
    for (name, list) in [
        ("COLOUR_ALLOW", COLOUR_ALLOW),
        ("HEX_ALLOW", HEX_ALLOW),
        ("FLOAT_ALLOW", FLOAT_ALLOW),
        ("OVERLAY_ALLOW", OVERLAY_ALLOW),
    ] {
        assert!(list.len() <= 5, "{name} exceeds the five-entry cap");
        for (suffix, needle, why) in list {
            assert!(
                !why.trim().is_empty(),
                "{name} entry {suffix}/{needle} carries no justification"
            );
        }
    }
}

/// **The one sanctioned overlay is sanctioned by its id, not by its file.**
///
/// An allowlist entry is a hole in a gate, and a hole the width of a file is a
/// different thing from a hole the width of one call. `OVERLAY_ALLOW`'s entry
/// names the hover readout's own id salt, so this asks the matcher directly:
/// the sanctioned line passes, and the same construction in the same file
/// under a different id does not.
///
/// Written because the entry's justification claims exactly this, and a
/// justification nothing checks is a sentence.
#[test]
fn a_second_hand_rolled_overlay_in_the_chart_pane_is_still_a_violation() {
    let sanctioned = r#"        egui::Area::new(egui::Id::new("chart-hover-readout"))"#;
    assert!(
        allowed(OVERLAY_ALLOW, "src/chart_item.rs", sanctioned),
        "the hover readout's own Area is not matched by the entry that exists \
         for it — the gate is red on the line it was widened for"
    );
    for other in [
        r#"        egui::Area::new(egui::Id::new("chart-second-overlay"))"#,
        r#"        egui::Window::new("chart-hover-readout")"#,
        r#"        let m = egui::Modal::new(egui::Id::new("chart-hover-readout"));"#,
    ] {
        assert!(
            !allowed(OVERLAY_ALLOW, "src/chart_item.rs", other),
            "the allowlist admits `{other}` — the entry is wider than the one \
             call it was written for"
        );
    }
    assert!(
        !allowed(OVERLAY_ALLOW, "src/inspector.rs", sanctioned),
        "the allowlist admits the readout's line in another file"
    );
}

/// **The shelf's card is sanctioned by its id, as the hover readout is.**
///
/// The second entry's justification claims it is as narrow as the first: this
/// asks the matcher directly, so the claim is checked and not only written.
#[test]
fn a_second_hand_rolled_overlay_in_the_shelf_is_still_a_violation() {
    let sanctioned = r#"        egui::Area::new(egui::Id::new("shelf-column-card"))"#;
    assert!(
        allowed(OVERLAY_ALLOW, "src/shelf.rs", sanctioned),
        "the shelf card's own Area is not matched by the entry that exists for it"
    );
    for other in [
        r#"        egui::Area::new(egui::Id::new("shelf-second-card"))"#,
        r#"        egui::Window::new("shelf-column-card")"#,
        r#"        let m = egui::Modal::new(egui::Id::new("shelf-column-card"));"#,
    ] {
        assert!(
            !allowed(OVERLAY_ALLOW, "src/shelf.rs", other),
            "the allowlist admits `{other}` in the shelf — the entry is wider \
             than the one call it was written for"
        );
    }
    assert!(
        !allowed(OVERLAY_ALLOW, "src/window.rs", sanctioned),
        "the allowlist admits the card's line in another file"
    );
}

/// **The statement form is sanctioned by its id, as the shelf's card is.**
#[test]
fn a_second_hand_rolled_overlay_beside_the_statement_form_is_still_a_violation() {
    let sanctioned = r#"        let shown = egui::Area::new(egui::Id::new(STATEMENT_CARD_ID))"#;
    assert!(
        allowed(OVERLAY_ALLOW, "src/typed_card.rs", sanctioned),
        "the statement form's own Area is not matched by the entry that exists for it"
    );
    for other in [
        r#"        egui::Area::new(egui::Id::new("bf-overlay-statement-2"))"#,
        r#"        egui::Area::new(id)"#,
        r#"        egui::Window::new(STATEMENT_CARD_ID)"#,
        r#"        let m = egui::Modal::new(egui::Id::new(STATEMENT_CARD_ID));"#,
    ] {
        assert!(
            !allowed(OVERLAY_ALLOW, "src/typed_card.rs", other),
            "the allowlist admits `{other}` beside the statement form — the entry \
             is wider than the one call it was written for"
        );
    }
    assert!(
        !allowed(OVERLAY_ALLOW, "src/window.rs", sanctioned),
        "the allowlist admits the form's line in another file"
    );
}

/// **The browser's sheet and the trust band are sanctioned by their ids, not by their file.**
/// `window.rs` holds both entries, so the same constructions under other ids, or the two ids in
/// another file, are still violations.
#[test]
fn a_second_hand_rolled_overlay_beside_the_browser_or_the_trust_band_is_still_a_violation() {
    let sanctioned = [
        r#"                let shown = egui::Modal::new(egui::Id::new("bf-overlay-sources-browser"))"#,
        r#"    egui::Area::new(egui::Id::new("bf-restricted-band"))"#,
    ];
    for line in sanctioned {
        assert!(
            allowed(OVERLAY_ALLOW, "src/window.rs", line),
            "the entry does not admit its own line `{line}`"
        );
        assert!(
            !allowed(OVERLAY_ALLOW, "src/overlays.rs", line),
            "the allowlist admits `{line}` in another file"
        );
    }
    for other in [
        r#"                let shown = egui::Modal::new(egui::Id::new("bf-overlay-other-sheet"))"#,
        r#"    egui::Area::new(egui::Id::new("bf-restricted-band-2"))"#,
        r#"    egui::Area::new(egui::Id::new("bf-overlay-sources-browser"))"#,
        r#"    egui::Window::new("bf-restricted-band")"#,
    ] {
        assert!(
            !allowed(OVERLAY_ALLOW, "src/window.rs", other),
            "the allowlist admits `{other}` in the window, wider than the two calls it names"
        );
    }
}

#[test]
fn the_shell_spells_no_colour_or_box_model_as_a_raw_literal() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    assert!(
        !files.is_empty(),
        "found no src/ files to scan under {manifest}"
    );

    let mut violations = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(manifest)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(file).expect("source file is UTF-8");
        for (n, line) in text.lines().enumerate() {
            // Token discipline governs code, not prose. A comment that *names* a
            // forbidden pattern — a doc comment describing the `add_space(6.0)`
            // calls it removed, say — is documentation, not a hand-typed
            // literal, and must not trip the gate.
            if line.trim_start().starts_with("//") {
                continue;
            }
            let loc = format!("{rel}:{}", n + 1);
            if line.contains("Color32::from_rgb(") && !allowed(COLOUR_ALLOW, &rel, line) {
                violations.push(format!(
                    "{loc}: `Color32::from_rgb(` — colour is a token, not raw channels"
                ));
            }
            if let Some(hex) = six_hex(line) {
                if !allowed(HEX_ALLOW, &rel, line) {
                    violations.push(format!("{loc}: six-hex colour literal `{hex}`"));
                }
            }
            for call in FLOAT_CALLS {
                if let Some(idx) = line.find(call) {
                    if has_bare_float(&line[idx..]) && !allowed(FLOAT_ALLOW, &rel, line) {
                        violations.push(format!(
                            "{loc}: bare float in `{call}` — use a spacing/radius token"
                        ));
                    }
                }
            }
            for needle in OVERLAY_NEEDLES {
                if line.contains(needle) && !allowed(OVERLAY_ALLOW, &rel, line) {
                    violations.push(format!(
                        "{loc}: `{needle}` — overlay chrome comes from meridian-egui's \
                         ModalLayer/NotificationLayer/ToastLayer, never hand-rolled here"
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "token-discipline violations ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}
