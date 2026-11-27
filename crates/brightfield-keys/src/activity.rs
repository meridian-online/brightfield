//! The activities of the node in focus: the verbs that show its rows, the
//! chart that reads it, and the model it is related in.
//!
//! They are the window's, as the ribbon's verbs are: what the canvas shows is
//! chrome neither document's model decides, so each is drawn over every
//! altitude a node can be in focus at.

/// The verb that shows the rows of the table or view in focus.
pub const SHOW_GRID: &str = "show-grid";
/// The verb that shows the chart that reads the table or view in focus.
pub const SHOW_CHART: &str = "show-chart";
/// The verb that shows the model that relates the Protocol's sources.
pub const RELATE: &str = "relate";

/// The verb that splits the canvas in two, the node in focus in one pane and
/// its other activity in the pane beside it, or closes the pane without focus.
pub const SPLIT_CANVAS: &str = "split-canvas";

/// The three activity verbs, in the order an activity switcher lists them.
pub const ACTIVITY_VERBS: [&str; 3] = [SHOW_GRID, SHOW_CHART, RELATE];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystroke::KeyPlatform;
    use crate::{registry, BindingContext, VerbStatus};

    #[test]
    fn each_activity_is_built_and_bound_once_to_command_option_and_its_initial() {
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
        assert_eq!(key(SHOW_GRID), "⌥⌘G");
        assert_eq!(key(SHOW_CHART), "⌥⌘C");
        assert_eq!(key(RELATE), "⌥⌘M");
        assert_eq!(key(SPLIT_CANVAS), "⌘\\");
    }

    #[test]
    fn relate_needs_two_sources_and_says_so() {
        let reg = registry();
        let relate = reg.iter().find(|v| v.longname == RELATE).unwrap();
        let one = crate::Situation {
            sources: 1,
            ..crate::Situation::default()
        };
        assert_eq!(
            crate::availability(relate, &one),
            crate::Availability::Missing("Relate needs two sources.".to_string())
        );
        let two = crate::Situation {
            sources: 2,
            ..crate::Situation::default()
        };
        assert_eq!(
            crate::availability(relate, &two),
            crate::Availability::Available
        );
    }
}
