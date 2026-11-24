//! The ribbon's taxonomy names only icons the design system carries, so no
//! command on a stage's tab or a contextual tab is drawn without its glyph.
//!
//! The operation forms' icons are checked where the forms join the ribbon.

use brightfield_keys::ribbon::{context_groups, stage_groups, Context, Stage};
use meridian_egui::Icon;

#[test]
fn every_ribbon_command_names_an_icon_the_design_system_carries() {
    let groups = Stage::ALL
        .into_iter()
        .flat_map(|stage| stage_groups(stage).iter())
        .chain(
            Context::ALL
                .into_iter()
                .flat_map(|context| context_groups(context).iter()),
        );
    let mut checked = 0;
    let mut missing = Vec::new();
    for group in groups {
        for entry in group.iter() {
            checked += 1;
            if Icon::by_name(entry.icon).is_none() {
                missing.push(format!("{} names {:?}", entry.longname, entry.icon));
            }
        }
    }
    assert!(checked > 0, "the taxonomy lists no commands to check");
    assert!(
        missing.is_empty(),
        "the design system's icon manifest carries none of these: {missing:?}"
    );
}
