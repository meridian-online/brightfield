//! Token discipline: this crate may not spell colour or the box model as raw
//! literals. Colour belongs to the Meridian scales, spacing to the ladder, radii
//! to the radius ladder — all reached through `meridian-egui`/`meridian-design`,
//! never invented inline. This test greps the crate's own `src/` for the four
//! ways that drift creeps back:
//!
//! - `Color32::from_rgb(` — a colour built from raw channels rather than a token
//!   (the sanctioned boundary is `to_color32`, which uses `from_rgba_unmultiplied`
//!   and is not matched by this precise, paren-anchored needle);
//! - a six-hex literal (`0xRRGGBB`) — a hand-typed colour;
//! - a bare float in `add_space(` / `Margin::` / `inner_margin(` / `Vec2::splat(`
//!   — a hand-typed gap or margin where a spacing/radius token belongs.
//!
//! It passes trivially today: an earlier change already moved every gap onto a
//! `spacing::` constant. It becomes load-bearing as more surfaces are built.
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
        }
    }

    assert!(
        violations.is_empty(),
        "token-discipline violations ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}
