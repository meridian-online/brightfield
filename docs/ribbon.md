# The ribbon

The ribbon is the window's row of commands for the stage of the work in hand. Its tabs sit in the title band after the Protocol's name, and the selected tab's commands sit in a band of their own under it. This doc says what each tab lists, how a command reads against what the window holds, and how a command that cannot act says why. Read it before changing the ribbon's tabs, its command row, or the way a refused key speaks.

## What it draws

- **Four stage tabs and a contextual tab.** The stages are Connect, Shape, Model and Publish, in that order. The contextual tab follows a divider and is named for what the canvas holds and the node it holds: `Grid · ranking` over a node's grid, `Chart · <dashboard>` over a chart, `Graph` or `Graph · <node>` over the graph, and `Model` over the model pane. The front door draws no ribbon.
- **The tabs stop short of Run and of the Protocol's name field.** The title band draws Run at its right end first and the tabs after the name, no wider than the room left before Run once the name field has its own, so a narrow window cuts the last tabs rather than covering either. A tab cut away is not recorded as a control, and a refused key's sentence is not anchored to it. The band measures the tabs' width on the frame they first say something new and lays its other controls out against it, as it does for the activity area.
- **The command row is its own region,** `RIBBON_BAND` in `brightfield_workbench::arrangement`, declared after the title band at the title band's height. It is a band that may be put away whole (`Collapse::Away`), which only a band may declare, since the tabs that bring it back are drawn elsewhere: a tab clicked while the ribbon is put away opens it on that tab; a rail is reopened from its own strip and so collapses onto it. The window it asks for and the panel tiles' first layout both count the band.
- **A tab lists groups of commands.** A stage tab leads with its built operation forms, in arc's order, so a form joins its tab the day it is built. The Grid tab leads with the forms its foot row asks for, the filter and the sort. The rest of each tab is `brightfield_keys::ribbon`'s taxonomy. A command a tab's forms already list is not listed again, and a group left empty is not drawn.

## How a command reads

`brightfield_shell::ribbon::ribbon_views` reads each command against the window: its label from the key registry, its icon, the keycap of its first binding that fires where the keys are now (`MeridianApp::dispatch_context`), and the sentence its declared needs give when the window does not meet them. A command whose key does not fire where the keys are shows no keycap, because a keycap that does nothing when pressed teaches the wrong key.

A click on a command goes the way a menu command goes: it joins the frame's requests as the verb and is performed by the same arm. The row takes no click on a greyed command.

## How a greyed command says why

- **Pointed at,** a greyed command opens its sentence in the reason popover under it.
- **Its key pressed,** a greyed command's handler refuses it (`MeridianApp::refuse_key`). The handler performs nothing and notes nothing in `keys_answered`, and the sentence is shown for four seconds: under the command when the window drew it (on the row, or Run in the title band), under its stage's tab when only the tab is drawn, and in an Info toast when neither is, as on the front door. `tests/ribbon.rs` presses every Global key of a command that is greyed on the front door and checks its sentence reaches the screen.

## Rulings made in the build

These were made while Hugh was away and can each be undone without changing what the ribbon offers.

- **Only Global keys are refused this way.** Workspace keys are bare keys on the chart canvas (the arrows, `e`, `u`, `1` to `3`, `r`), and several are read by other handlers when their own command cannot act. Saying a sentence for each of them would put a popover or a toast on every arrow pressed over the graph, and could take a key another handler wants. They are left as they were.
- **F2 on the front door still renames the row under the pointer,** though Rename the Protocol needs a Protocol open; it is refused only where it would act on nothing.
- **⌘S is not refused while an operation is being made,** since Save keeps the operation first whatever else is left to write.
- **The reason's time is the window's frame clock,** not the wall clock, so a test's frames decide when it comes down.
- **The command row's region draws in the rail frame,** which has no margin: the row draws its own 32-point band, and a band margin would push it past the height the arrangement declares.
- **A ribbon command named by both a form and the taxonomy** (the sources browser, on Connect) is listed once, with the forms.
