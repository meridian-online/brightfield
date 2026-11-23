//! A binding's keystrokes, parsed into strokes and written as the label a menu
//! or a help row shows.
//!
//! The registry writes a binding as space-separated strokes, each a key with
//! its modifiers in front: `"cmd-shift-p"`, `"shift-tab"`, `"d d"`. This module
//! reads that form into [`Keystroke`]s and labels each one per platform, so a
//! menu or a help row shows the registry's own key.

/// One stroke of a binding: a key and the modifiers held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keystroke {
    /// The key's name, with its modifiers taken off. A registry binding's is
    /// one of [`KEY_NAMES`] (`every_registry_binding_parses_to_a_known_key`).
    pub key: &'static str,
    /// The platform's command modifier: ⌘ on the Mac and Ctrl elsewhere.
    pub command: bool,
    /// The Control key itself.
    pub control: bool,
    /// Option on the Mac, Alt elsewhere.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
}

/// Which platform's conventions a label follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPlatform {
    /// macOS: modifier glyphs in Apple's order, with no separator.
    Mac,
    /// Linux and Windows: modifier names joined by `+`.
    Other,
}

impl KeyPlatform {
    /// The platform this build runs on: [`KeyPlatform::Mac`] on macOS and
    /// [`KeyPlatform::Other`] elsewhere.
    #[must_use]
    pub const fn host() -> Self {
        if cfg!(target_os = "macos") {
            KeyPlatform::Mac
        } else {
            KeyPlatform::Other
        }
    }
}

/// Every key name a binding may use
/// (`every_registry_binding_parses_to_a_known_key`).
pub const KEY_NAMES: &[&str] = &[
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "enter",
    "escape",
    "tab",
    "backspace",
    "space",
    "left",
    "right",
    "up",
    "down",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "/",
    "?",
    "=",
    "-",
    ",",
];

/// Parse a binding's space-separated keystrokes into strokes.
///
/// A stroke's modifiers come off the front while one matches and leaves a key
/// behind it, so `-` alone is the minus key and `cmd--` is ⌘ with it.
#[must_use]
pub fn parse_keystrokes(keystrokes: &'static str) -> Vec<Keystroke> {
    keystrokes.split_whitespace().map(parse_stroke).collect()
}

fn parse_stroke(stroke: &'static str) -> Keystroke {
    let mut parsed = Keystroke {
        key: stroke,
        command: false,
        control: false,
        alt: false,
        shift: false,
    };
    loop {
        let rest = parsed.key;
        let (held, key): (&mut bool, &'static str) = if let Some(key) = rest.strip_prefix("cmd-") {
            (&mut parsed.command, key)
        } else if let Some(key) = rest.strip_prefix("ctrl-") {
            (&mut parsed.control, key)
        } else if let Some(key) = rest.strip_prefix("alt-") {
            (&mut parsed.alt, key)
        } else if let Some(key) = rest.strip_prefix("shift-") {
            (&mut parsed.shift, key)
        } else {
            break;
        };
        if key.is_empty() {
            break;
        }
        *held = true;
        parsed.key = key;
    }
    parsed
}

impl Keystroke {
    /// The stroke as a menu shows it: `⇧⌘P` on the Mac, `Ctrl+Shift+P`
    /// elsewhere.
    #[must_use]
    pub fn label(&self, platform: KeyPlatform) -> String {
        let key = key_label(self.key, platform);
        match platform {
            // Apple's order is ⌃ ⌥ ⇧ ⌘, then the key.
            KeyPlatform::Mac => {
                let mut label = String::new();
                for (held, glyph) in [
                    (self.control, '⌃'),
                    (self.alt, '⌥'),
                    (self.shift, '⇧'),
                    (self.command, '⌘'),
                ] {
                    if held {
                        label.push(glyph);
                    }
                }
                label.push_str(&key);
                label
            }
            // The command modifier is Ctrl here, so a stroke holding both it
            // and Control names Ctrl once.
            KeyPlatform::Other => {
                let mut parts: Vec<&str> = Vec::new();
                if self.command || self.control {
                    parts.push("Ctrl");
                }
                if self.alt {
                    parts.push("Alt");
                }
                if self.shift {
                    parts.push("Shift");
                }
                parts.push(&key);
                parts.join("+")
            }
        }
    }
}

/// A key's own label: a letter in upper case, a named key as its glyph on the
/// Mac and its name elsewhere, punctuation as itself.
fn key_label(key: &str, platform: KeyPlatform) -> String {
    let mac = platform == KeyPlatform::Mac;
    let named = match key {
        "enter" => Some(if mac { "↩" } else { "Enter" }),
        "escape" => Some(if mac { "⎋" } else { "Esc" }),
        "tab" => Some(if mac { "⇥" } else { "Tab" }),
        "backspace" => Some(if mac { "⌫" } else { "Backspace" }),
        "space" => Some("Space"),
        "left" => Some("←"),
        "right" => Some("→"),
        "up" => Some("↑"),
        "down" => Some("↓"),
        _ => None,
    };
    match named {
        Some(label) => label.to_string(),
        // Letters and the function keys (`f2` → `F2`); digits and punctuation
        // have no upper case and stay as they are.
        None => key.to_uppercase(),
    }
}

/// A binding's whole label: each stroke's, joined by a space (`"d d"` → `D D`).
#[must_use]
pub fn keys_label(keystrokes: &'static str, platform: KeyPlatform) -> String {
    parse_keystrokes(keystrokes)
        .iter()
        .map(|stroke| stroke.label(platform))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: &'static str) -> Keystroke {
        Keystroke {
            key,
            command: false,
            control: false,
            alt: false,
            shift: false,
        }
    }

    #[test]
    fn modifiers_come_off_the_front_and_the_rest_is_the_key() {
        assert_eq!(
            parse_keystrokes("cmd-shift-p"),
            [Keystroke {
                command: true,
                shift: true,
                ..plain("p")
            }]
        );
        // zoom-out's key is the minus itself.
        assert_eq!(parse_keystrokes("-"), [plain("-")]);
        assert_eq!(
            parse_keystrokes("cmd--"),
            [Keystroke {
                command: true,
                ..plain("-")
            }]
        );
        assert_eq!(parse_keystrokes("cmd-,")[0].key, ",");
        assert_eq!(
            parse_keystrokes("shift-tab")[0],
            Keystroke {
                shift: true,
                ..plain("tab")
            }
        );
        assert_eq!(
            parse_keystrokes("ctrl-alt-x")[0],
            Keystroke {
                control: true,
                alt: true,
                ..plain("x")
            }
        );
        assert_eq!(parse_keystrokes("d d"), [plain("d"), plain("d")]);
    }

    #[test]
    fn mac_labels_use_the_menu_glyphs_in_apples_order() {
        // Apple's order is ⌃ ⌥ ⇧ ⌘, then the key.
        for (keys, want) in [
            ("cmd-shift-p", "⇧⌘P"),
            ("shift-cmd-p", "⇧⌘P"),
            ("ctrl-alt-shift-cmd-x", "⌃⌥⇧⌘X"),
            ("cmd-enter", "⌘↩"),
            ("shift-t", "⇧T"),
            ("shift-tab", "⇧⇥"),
            ("escape", "⎋"),
            ("backspace", "⌫"),
            ("space", "Space"),
            ("left", "←"),
            ("f2", "F2"),
            ("?", "?"),
            ("-", "-"),
            ("0", "0"),
            ("cmd-,", "⌘,"),
            ("d d", "D D"),
        ] {
            assert_eq!(keys_label(keys, KeyPlatform::Mac), want, "{keys}");
        }
    }

    #[test]
    fn other_labels_spell_ctrl_and_the_key_names() {
        for (keys, want) in [
            ("cmd-shift-p", "Ctrl+Shift+P"),
            ("cmd-enter", "Ctrl+Enter"),
            ("ctrl-cmd-x", "Ctrl+X"),
            ("alt-shift-x", "Alt+Shift+X"),
            ("escape", "Esc"),
            ("shift-tab", "Shift+Tab"),
            ("backspace", "Backspace"),
            ("shift-t", "Shift+T"),
            ("-", "-"),
            ("g t", "G T"),
        ] {
            assert_eq!(keys_label(keys, KeyPlatform::Other), want, "{keys}");
        }
    }

    #[test]
    fn every_registry_binding_parses_to_a_known_key() {
        let mut bindings = 0;
        for v in crate::registry() {
            for b in &v.binding_specs {
                let strokes = parse_keystrokes(b.keystrokes);
                assert!(!strokes.is_empty(), "{} binds no key", v.longname);
                for s in strokes {
                    bindings += 1;
                    assert!(
                        KEY_NAMES.contains(&s.key),
                        "{} binds unknown key {:?}",
                        v.longname,
                        s.key
                    );
                }
            }
        }
        assert!(bindings > 0, "the registry binds no key at all");
    }
}
