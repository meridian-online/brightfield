//! The ribbon's taxonomy: its four stage tabs, its contextual tab for what the
//! canvas holds, and the commands each lists beyond the operation forms.
//!
//! A stage tab's first group is the built operation forms of that stage, which
//! the shell reads from its own registry of forms, so a form joins its tab when
//! it is built and this file does not change. What is listed here is every other
//! command a tab shows, in groups, each with its Tabler icon. No operation verb
//! is listed here (`the_ribbons_own_lists_hold_no_operation_verb`), and every
//! command listed is built (`the_ribbon_lists_only_built_commands`).

/// A stage of the work, and the ribbon tab that holds its commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Connecting to data where it lives.
    Connect,
    /// Shaping one table.
    Shape,
    /// Relating tables.
    Model,
    /// Taking a chart out of the window.
    Publish,
}

impl Stage {
    /// The four stages, in the order their tabs are drawn.
    pub const ALL: [Stage; 4] = [Stage::Connect, Stage::Shape, Stage::Model, Stage::Publish];

    /// The tab's name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Stage::Connect => "Connect",
            Stage::Shape => "Shape",
            Stage::Model => "Model",
            Stage::Publish => "Publish",
        }
    }

    /// The verb whose key selects this stage's tab.
    #[must_use]
    pub const fn tab_verb(self) -> &'static str {
        match self {
            Stage::Connect => SHOW_CONNECT_TAB,
            Stage::Shape => SHOW_SHAPE_TAB,
            Stage::Model => SHOW_MODEL_TAB,
            Stage::Publish => SHOW_PUBLISH_TAB,
        }
    }
}

/// What the canvas holds, which names the contextual tab and fills it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    /// A table's rows in the grid.
    Grid,
    /// A chart.
    Chart,
    /// The model of the Protocol's tables.
    Model,
    /// The Protocol's graph.
    Graph,
}

impl Context {
    /// Every context, in no order that is drawn.
    pub const ALL: [Context; 4] = [
        Context::Grid,
        Context::Chart,
        Context::Model,
        Context::Graph,
    ];

    /// The contextual tab's name while the canvas holds this.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Context::Grid => "Grid",
            Context::Chart => "Chart",
            Context::Model => "Model",
            Context::Graph => "Graph",
        }
    }
}

/// One command on the ribbon: its verb and its Tabler icon's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RibbonEntry {
    /// The registry verb the command performs.
    pub longname: &'static str,
    /// The Tabler icon drawn for it.
    pub icon: &'static str,
}

/// A run of commands drawn together, between dividers.
pub type Group = &'static [RibbonEntry];

/// The verb that selects the Connect tab.
pub const SHOW_CONNECT_TAB: &str = "show-connect-tab";
/// The verb that selects the Shape tab.
pub const SHOW_SHAPE_TAB: &str = "show-shape-tab";
/// The verb that selects the Model tab.
pub const SHOW_MODEL_TAB: &str = "show-model-tab";
/// The verb that selects the Publish tab.
pub const SHOW_PUBLISH_TAB: &str = "show-publish-tab";
/// The verb that selects the contextual tab.
pub const SHOW_CONTEXT_TAB: &str = "show-context-tab";
/// The verb that collapses the ribbon to its tabs, or opens it again.
pub const TOGGLE_RIBBON: &str = "toggle-ribbon";

/// The six verbs that move about the ribbon.
pub const RIBBON_VERBS: [&str; 6] = [
    SHOW_CONNECT_TAB,
    SHOW_SHAPE_TAB,
    SHOW_MODEL_TAB,
    SHOW_PUBLISH_TAB,
    SHOW_CONTEXT_TAB,
    TOGGLE_RIBBON,
];

const fn entry(longname: &'static str, icon: &'static str) -> RibbonEntry {
    RibbonEntry { longname, icon }
}

const CONNECT: &[Group] = &[&[
    entry("open", "folder"),
    entry("open-sources-browser", "database"),
]];
const SHAPE: &[Group] = &[];
const MODEL: &[Group] = &[&[
    entry("organise-joins", "hierarchy"),
    entry("relate", "arrows-join"),
]];
const PUBLISH: &[Group] = &[&[
    entry("copy-chart-picture", "copy"),
    entry("export-chart-picture", "download"),
]];

const GRID: &[Group] = &[];
const CHART: &[Group] = &[
    &[
        entry("set-channel", "adjustments"),
        entry("put-column-on-x", "axis-x"),
        entry("put-column-on-y", "axis-y"),
        entry("put-column-on-colour", "palette"),
    ],
    &[
        entry("zoom-in", "zoom-in"),
        entry("zoom-out", "zoom-out"),
        entry("reset-extent", "maximize"),
        entry("cycle-axis-lock", "lock"),
    ],
    &[entry("undo", "arrow-back-up")],
];
const MODEL_VIEW: &[Group] = &[
    &[
        entry("show-model-diagram", "chart-dots"),
        entry("show-model-list", "list"),
        entry("show-model-file", "file"),
    ],
    &[
        entry("add-join", "link"),
        entry("keep-join", "check"),
        entry("drop-join", "unlink"),
        entry("change-join-keys", "key"),
    ],
    &[entry("rediscover-model", "refresh")],
];
const GRAPH: &[Group] = &[
    &[
        entry("toggle-fold", "fold"),
        entry("open-steps-sheet", "list"),
    ],
    &[entry("yank-address", "copy"), entry("remove-step", "trash")],
];

/// The groups a stage's tab lists after its operation forms.
#[must_use]
pub const fn stage_groups(stage: Stage) -> &'static [Group] {
    match stage {
        Stage::Connect => CONNECT,
        Stage::Shape => SHAPE,
        Stage::Model => MODEL,
        Stage::Publish => PUBLISH,
    }
}

/// The groups the contextual tab lists after the forms asked from there.
#[must_use]
pub const fn context_groups(context: Context) -> &'static [Group] {
    match context {
        Context::Grid => GRID,
        Context::Chart => CHART,
        Context::Model => MODEL_VIEW,
        Context::Graph => GRAPH,
    }
}

/// The stage whose tab lists `longname` in this taxonomy. An operation form's
/// stage is its own and is not answered here.
#[must_use]
pub fn stage_of(longname: &str) -> Option<Stage> {
    Stage::ALL.into_iter().find(|&stage| {
        stage_groups(stage)
            .iter()
            .any(|group| group.iter().any(|e| e.longname == longname))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystroke::KeyPlatform;
    use crate::{operations, registry, BindingContext, VerbStatus};

    fn every_entry() -> impl Iterator<Item = &'static RibbonEntry> {
        Stage::ALL
            .into_iter()
            .flat_map(|s| stage_groups(s).iter())
            .chain(
                Context::ALL
                    .into_iter()
                    .flat_map(|c| context_groups(c).iter()),
            )
            .flat_map(|group| group.iter())
    }

    #[test]
    fn the_ribbon_lists_only_built_commands() {
        let reg = registry();
        for entry in every_entry() {
            let verb = reg
                .iter()
                .find(|v| v.longname == entry.longname)
                .unwrap_or_else(|| panic!("{} is not registered", entry.longname));
            assert_eq!(verb.status, VerbStatus::Built, "{}", entry.longname);
        }
    }

    #[test]
    fn joining_sits_under_model() {
        assert_eq!(stage_of("organise-joins"), Some(Stage::Model));
        assert_eq!(stage_of("relate"), Some(Stage::Model));
    }

    #[test]
    fn a_command_on_no_stage_tab_has_no_stage() {
        assert_eq!(stage_of("zoom-in"), None);
        assert_eq!(stage_of("filter-rows-at-cursor"), None);
    }

    #[test]
    fn the_ribbons_own_lists_hold_no_operation_verb() {
        for entry in every_entry() {
            assert!(
                !operations::is_operation_verb(entry.longname)
                    && !matches!(
                        entry.longname,
                        "filter-rows-at-cursor" | "sort-rows-at-cursor"
                    ),
                "{} is an operation's verb, which the forms list",
                entry.longname
            );
        }
    }

    #[test]
    fn every_tab_has_a_bound_key_of_its_own() {
        let reg = registry();
        let tab_verbs: Vec<&str> = Stage::ALL
            .into_iter()
            .map(Stage::tab_verb)
            .chain([SHOW_CONTEXT_TAB])
            .collect();
        let mut keys = Vec::new();
        for longname in &tab_verbs {
            let verb = reg
                .iter()
                .find(|v| v.longname == *longname)
                .unwrap_or_else(|| panic!("{longname} is not registered"));
            assert_eq!(verb.status, VerbStatus::Built, "{longname}");
            assert_eq!(verb.binding_specs.len(), 1, "{longname} is bound once");
            assert_eq!(verb.binding_specs[0].context, BindingContext::Global);
            keys.push(verb.binding_specs[0].keystrokes);
        }
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "two tabs share a key: {keys:?}");
    }

    #[test]
    fn each_ribbon_verb_is_labelled_and_needs_a_protocol() {
        let reg = registry();
        for longname in RIBBON_VERBS {
            let verb = reg
                .iter()
                .find(|v| v.longname == longname)
                .unwrap_or_else(|| panic!("{longname} is not registered"));
            assert!(!verb.label.is_empty(), "{longname}");
            assert!(
                verb.needs
                    .iter()
                    .any(|r| r.need == crate::Need::Selected(crate::Selected::Protocol)),
                "{longname} draws nothing on the front door, so it needs a Protocol"
            );
        }
    }

    #[test]
    fn the_tabs_keys_read_as_option_and_the_tabs_initial() {
        let reg = registry();
        let key = |longname: &str| {
            reg.iter()
                .find(|v| v.longname == longname)
                .and_then(crate::VerbEntry::primary_key)
                .map(|k| crate::keys_label(k, KeyPlatform::Mac))
        };
        assert_eq!(key(SHOW_CONNECT_TAB).as_deref(), Some("⌥C"));
        assert_eq!(key(SHOW_SHAPE_TAB).as_deref(), Some("⌥S"));
        assert_eq!(key(SHOW_MODEL_TAB).as_deref(), Some("⌥M"));
        assert_eq!(key(SHOW_PUBLISH_TAB).as_deref(), Some("⌥P"));
        assert_eq!(key(SHOW_CONTEXT_TAB).as_deref(), Some("⌥X"));
        assert_eq!(key(TOGGLE_RIBBON).as_deref(), Some("⌥R"));
    }

    #[test]
    fn the_host_platform_is_the_mac_on_macos() {
        assert_eq!(
            KeyPlatform::host() == KeyPlatform::Mac,
            cfg!(target_os = "macos")
        );
    }
}
