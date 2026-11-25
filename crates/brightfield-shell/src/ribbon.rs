//! The window's ribbon: which tab is selected and whether it is collapsed.
//!
//! What each tab lists is [`brightfield_keys::ribbon`]'s, and the operation
//! forms' stages are [`crate::operation_ui`]'s. This is the part that changes
//! as the analyst works: the tab their keys or clicks picked, and the collapse.

use brightfield_keys::ribbon::{
    Stage, SHOW_CONNECT_TAB, SHOW_CONTEXT_TAB, SHOW_MODEL_TAB, SHOW_PUBLISH_TAB, SHOW_SHAPE_TAB,
    TOGGLE_RIBBON,
};

/// The ribbon tab that is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonPick {
    /// One of the four stage tabs.
    Stage(Stage),
    /// The tab named for what the canvas holds.
    Context,
}

/// The ribbon as the analyst left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RibbonState {
    /// The selected tab. It opens on Shape and moves only when the analyst
    /// moves it: switching the canvas renames the contextual tab and leaves
    /// this where it was.
    pub selected: RibbonPick,
    /// Whether the ribbon shows its tabs alone.
    pub collapsed: bool,
}

impl Default for RibbonState {
    fn default() -> Self {
        Self {
            selected: RibbonPick::Stage(Stage::Shape),
            collapsed: false,
        }
    }
}

impl RibbonState {
    /// Perform one of the ribbon's verbs. Answers whether `verb` is one.
    pub fn perform(&mut self, verb: &str) -> bool {
        let pick = match verb {
            SHOW_CONNECT_TAB => RibbonPick::Stage(Stage::Connect),
            SHOW_SHAPE_TAB => RibbonPick::Stage(Stage::Shape),
            SHOW_MODEL_TAB => RibbonPick::Stage(Stage::Model),
            SHOW_PUBLISH_TAB => RibbonPick::Stage(Stage::Publish),
            SHOW_CONTEXT_TAB => RibbonPick::Context,
            TOGGLE_RIBBON => {
                self.collapsed = !self.collapsed;
                return true;
            }
            _ => return false,
        };
        self.selected = pick;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_keys::ribbon::RIBBON_VERBS;

    #[test]
    fn the_ribbon_opens_on_shape_and_open() {
        let state = RibbonState::default();
        assert_eq!(state.selected, RibbonPick::Stage(Stage::Shape));
        assert!(!state.collapsed);
    }

    #[test]
    fn each_tab_verb_selects_its_tab_and_leaves_the_collapse() {
        let mut state = RibbonState::default();
        for stage in Stage::ALL {
            assert!(state.perform(stage.tab_verb()));
            assert_eq!(state.selected, RibbonPick::Stage(stage));
        }
        assert!(state.perform(SHOW_CONTEXT_TAB));
        assert_eq!(state.selected, RibbonPick::Context);
        assert!(!state.collapsed);
    }

    #[test]
    fn the_toggle_collapses_and_opens_and_keeps_the_tab() {
        let mut state = RibbonState::default();
        state.perform(SHOW_MODEL_TAB);
        state.perform(TOGGLE_RIBBON);
        assert!(state.collapsed);
        state.perform(TOGGLE_RIBBON);
        assert!(!state.collapsed);
        assert_eq!(state.selected, RibbonPick::Stage(Stage::Model));
    }

    #[test]
    fn every_ribbon_verb_is_performed_and_no_other() {
        let mut state = RibbonState::default();
        for verb in RIBBON_VERBS {
            assert!(state.perform(verb), "{verb}");
        }
        assert!(!state.perform("zoom-in"));
    }
}
