//! **The open places' verbs**: walking the list of places above the spine,
//! and closing one.

/// The verb that makes the place below the current one current.
pub const NEXT_PLACE: &str = "next-place";
/// The verb that makes the place above the current one current.
pub const PREV_PLACE: &str = "prev-place";
/// The verb that closes the current place.
pub const CLOSE_PLACE: &str = "close-place";

/// The three place verbs, in the order the Window menu lists them.
pub const PLACE_VERBS: [&str; 3] = [NEXT_PLACE, PREV_PLACE, CLOSE_PLACE];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystroke::KeyPlatform;
    use crate::{registry, BindingContext, VerbStatus};

    #[test]
    fn each_place_verb_is_built_and_bound_once_globally() {
        let reg = registry();
        let key = |longname: &str| {
            let verb = reg
                .iter()
                .find(|v| v.longname == longname)
                .unwrap_or_else(|| panic!("{longname} is not registered"));
            assert_eq!(verb.status, VerbStatus::Built, "{longname}");
            assert_eq!(verb.binding_specs.len(), 1, "{longname} is bound once");
            assert_eq!(verb.binding_specs[0].context, BindingContext::Global);
            crate::keys_label(verb.binding_specs[0].keystrokes, KeyPlatform::Mac)
        };
        assert_eq!(key(NEXT_PLACE), "⇧⌘]");
        assert_eq!(key(PREV_PLACE), "⇧⌘[");
        assert_eq!(key(CLOSE_PLACE), "⌘W");
    }
}
