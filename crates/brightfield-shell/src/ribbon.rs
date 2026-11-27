//! The window's ribbon: which tab is selected, whether it is collapsed, and
//! what the selected tab's command row lists.
//!
//! What each tab lists beyond the operation forms is
//! [`brightfield_keys::ribbon`]'s, and the forms and their stages are
//! [`crate::operation_ui`]'s. A stage tab leads with its built forms, so a form
//! joins its tab when it is built. [`ribbon_views`] reads each command against
//! what the window holds: its label, its icon, the key that performs it where
//! the keys are now, and the sentence it says when it cannot act.

use brightfield_keys::ribbon::{
    context_groups, stage_groups, Context, RibbonEntry, Stage, SHOW_CONNECT_TAB, SHOW_CONTEXT_TAB,
    SHOW_MODEL_TAB, SHOW_PUBLISH_TAB, SHOW_SHAPE_TAB, TOGGLE_RIBBON,
};
use brightfield_keys::{
    availability, fires, keys_label, registry, Availability, DispatchContext, KeyPlatform,
    Situation,
};

use crate::operation_ui::{self, Start};

/// How long a greyed command's sentence stays up after its key was refused, in
/// seconds of the window's frame clock.
pub const REASON_SHOWN_FOR: f64 = 4.0;

/// A greyed command's sentence, shown because its key was pressed where the
/// command cannot act.
#[derive(Clone, Debug, PartialEq)]
pub struct ReasonShown {
    /// The command refused.
    pub longname: &'static str,
    /// What it needs, as its declaration says it.
    pub sentence: String,
    /// When it comes down, on the window's frame clock.
    pub until: f64,
}

/// One command as the row draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandView {
    /// The registry verb it performs.
    pub longname: &'static str,
    /// Its name, the registry's label.
    pub label: &'static str,
    /// Its Tabler icon's name.
    pub icon: &'static str,
    /// The key that performs it where the keys are now, or `None` when none
    /// of its bindings fires there.
    pub keycap: Option<String>,
    /// The sentence it says when it cannot act, or `None` when it can.
    pub unavailable: Option<String>,
}

/// The groups a stage's tab lists: first its built operation forms, in arc's
/// order, then the taxonomy's own groups. A command the forms already list is
/// not listed again, and an empty group is dropped.
#[must_use]
pub fn stage_commands(stage: Stage) -> Vec<Vec<RibbonEntry>> {
    let forms = operation_ui::built()
        .filter(|form| form.stage == stage)
        .map(|form| RibbonEntry {
            longname: form.verb,
            icon: form.icon,
        })
        .collect();
    with_groups(forms, stage_groups(stage))
}

/// The groups the contextual tab lists while the canvas holds `context`: for a
/// grid, first the built forms asked from the grid's foot row, then the
/// taxonomy's own groups for that context.
#[must_use]
pub fn context_commands(context: Context) -> Vec<Vec<RibbonEntry>> {
    let forms = if context == Context::Grid {
        operation_ui::built()
            .filter(|form| {
                form.starts
                    .iter()
                    .any(|start| matches!(start, Start::FootField(_)))
            })
            .map(|form| RibbonEntry {
                longname: form.verb,
                icon: form.icon,
            })
            .collect()
    } else {
        Vec::new()
    };
    with_groups(forms, context_groups(context))
}

/// `forms` as the first group, then `groups` with what `forms` holds left out,
/// dropping any group left empty.
fn with_groups(
    forms: Vec<RibbonEntry>,
    groups: &[brightfield_keys::ribbon::Group],
) -> Vec<Vec<RibbonEntry>> {
    let rest = groups.iter().map(|group| {
        group
            .iter()
            .filter(|entry| !forms.iter().any(|f| f.longname == entry.longname))
            .copied()
            .collect::<Vec<_>>()
    });
    std::iter::once(forms.clone())
        .chain(rest)
        .filter(|group| !group.is_empty())
        .collect()
}

/// Each command of `groups` read against what the window holds: its label, its
/// keycap where the keys are (`dispatch`), and its sentence when `situation`
/// does not meet its needs. A longname the registry lacks is left out.
#[must_use]
pub fn ribbon_views(
    groups: &[Vec<RibbonEntry>],
    situation: &Situation,
    dispatch: DispatchContext,
    platform: KeyPlatform,
) -> Vec<Vec<CommandView>> {
    let reg = registry();
    groups
        .iter()
        .map(|group| {
            group
                .iter()
                .filter_map(|entry| {
                    let verb = reg.iter().find(|v| v.longname == entry.longname)?;
                    let keycap = verb
                        .binding_specs
                        .iter()
                        .find(|b| fires(b.context, dispatch))
                        .map(|b| keys_label(b.keystrokes, platform));
                    let unavailable = match availability(verb, situation) {
                        Availability::Available => None,
                        Availability::Missing(sentence) => Some(sentence),
                    };
                    Some(CommandView {
                        longname: verb.longname,
                        label: verb.label,
                        icon: entry.icon,
                        keycap,
                        unavailable,
                    })
                })
                .collect()
        })
        .collect()
}

/// The stage tab that lists `longname`: a form's own stage, or the stage the
/// taxonomy files it under. `None` for a command on no stage tab.
#[must_use]
pub fn stage_listing(longname: &str) -> Option<Stage> {
    operation_ui::by_verb(longname)
        .map(|form| form.stage)
        .or_else(|| brightfield_keys::ribbon::stage_of(longname))
}

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

    fn longnames(group: &[RibbonEntry]) -> Vec<&'static str> {
        group.iter().map(|e| e.longname).collect()
    }

    #[test]
    fn the_shape_tab_leads_with_the_built_forms_in_arcs_order() {
        let verbs: Vec<_> = operation_ui::built()
            .filter(|f| f.stage == Stage::Shape)
            .map(|f| f.verb)
            .collect();
        assert_eq!(longnames(&stage_commands(Stage::Shape)[0]), verbs);
        assert_eq!(verbs[..2], ["filter-rows-at-cursor", "sort-rows-at-cursor"]);
    }

    #[test]
    fn a_form_the_taxonomy_also_lists_is_listed_once() {
        let listed: Vec<_> = stage_commands(Stage::Connect)
            .iter()
            .flat_map(|g| longnames(g))
            .collect();
        let mut once = listed.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), listed.len(), "{listed:?}");
        assert!(listed.contains(&"open-sources-browser"));
        assert!(listed.contains(&"open"));
    }

    #[test]
    fn no_tab_lists_an_empty_group() {
        for groups in Stage::ALL
            .map(stage_commands)
            .into_iter()
            .chain(Context::ALL.map(context_commands))
        {
            assert!(groups.iter().all(|g| !g.is_empty()), "{groups:?}");
        }
    }

    #[test]
    fn the_grid_tab_leads_with_the_forms_its_foot_row_asks() {
        assert_eq!(
            longnames(&context_commands(Context::Grid)[0]),
            ["filter-rows-at-cursor", "sort-rows-at-cursor"]
        );
    }

    #[test]
    fn a_command_whose_key_does_not_fire_here_shows_no_keycap() {
        let views = ribbon_views(
            &stage_commands(Stage::Shape),
            &Situation::default(),
            DispatchContext::CanvasFocused,
            KeyPlatform::Mac,
        );
        // `w` binds in the Grid context only.
        assert_eq!(views[0][0].keycap, None);
    }

    #[test]
    fn a_greyed_command_carries_the_sentence_its_needs_declare() {
        let views = ribbon_views(
            &stage_commands(Stage::Shape),
            &Situation::default(),
            DispatchContext::GridFocused,
            KeyPlatform::Mac,
        );
        assert_eq!(views[0][0].keycap.as_deref(), Some("W"));
        let reg = registry();
        let filter = reg
            .iter()
            .find(|v| v.longname == "filter-rows-at-cursor")
            .expect("the filter is registered");
        assert_eq!(
            views[0][0].unavailable,
            availability(filter, &Situation::default())
                .missing()
                .map(str::to_owned)
        );
        assert!(views[0][0].unavailable.is_some());
    }

    #[test]
    fn a_command_whose_needs_are_met_is_not_greyed() {
        let reg = registry();
        let open = reg.iter().find(|v| v.longname == "open").expect("open");
        let views = ribbon_views(
            &stage_commands(Stage::Connect),
            &Situation::meeting(open.needs),
            DispatchContext::CanvasFocused,
            KeyPlatform::Mac,
        );
        let open = views
            .iter()
            .flatten()
            .find(|v| v.longname == "open")
            .expect("Connect lists open");
        assert_eq!(open.unavailable, None);
        assert_eq!(open.keycap.as_deref(), Some("⌘O"));
    }

    #[test]
    fn every_built_form_names_an_icon_the_design_system_carries() {
        let missing: Vec<_> = operation_ui::built()
            .filter(|f| meridian_egui::Icon::by_name(f.icon).is_none())
            .map(|f| format!("{} names {:?}", f.verb, f.icon))
            .collect();
        assert!(operation_ui::built().count() > 0);
        assert!(
            missing.is_empty(),
            "the design system's icon manifest carries none of these: {missing:?}"
        );
    }

    #[test]
    fn a_form_is_listed_under_its_own_stage_and_a_taxonomy_command_under_its_tab() {
        assert_eq!(stage_listing("filter-rows-at-cursor"), Some(Stage::Shape));
        assert_eq!(stage_listing("relate"), Some(Stage::Model));
        assert_eq!(stage_listing("zoom-in"), None);
    }

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
