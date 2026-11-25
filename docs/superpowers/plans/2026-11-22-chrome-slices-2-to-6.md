# Brightfield's chrome, slices 2 to 6: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Assemble the new chrome from components that `meridian-online/design` builds for it. The chrome is the unified title band and the ribbon, the spine's three widths with the open places and quick open, the activity switcher and the split, the one inspector, and the console with a status band that teaches keys.

**Architecture:** Each slice first pins the design repo's rev that carries its components, in `[workspace.dependencies]` of the root `Cargo.toml`. It then redraws one region:
- the shell's draw path in `crates/brightfield-shell/src/window.rs`;
- the workbench's one drawing file, `crates/brightfield-workbench/src/chrome.rs`;
- the regions declared in `crates/brightfield-workbench/src/arrangement.rs`;
- every command and key in `crates/brightfield-keys`.

Work that needs no new component gets its own task, so it can land before the components do. That covers the ribbon's taxonomy, run progress from arc's live stream, step timings, quick open, activities, column profiles, the console's panes and the status band's region. Brightfield draws no stand-in for a component that has not landed.

**Tech Stack:**
- Rust 1.95, egui/eframe 0.35, egui_kittest.
- `meridian-egui` and `meridian-design`, git-pinned.
- DuckDB, through `brightfield-engine`.
- arc's run record and `.jsonl` stream, through `brightfield-protocol`.
- chrono.

**Spec:** `docs/superpowers/specs/2026-11-21-brightfield-chrome-design.md`. Slice 1 (availability declarations and the native menu bar) and the operation forms are planned separately. This plan consumes slice 1 as an interface; slice 1 is built and its plan removed, so the interface is the code and the README's "Commands, menus and keys". See "What this plan needs from slice 1". It reads the operation forms from their registry (`docs/superpowers/plans/2026-11-22-operation-forms.md`), so no form task edits a file of this plan's.

## Global Constraints

**Regions**
- The region kinds are fixed: "bands, rails, one canvas and overlays. A band has a fixed height and full width and is never resized. A rail has a default and minimum extent, resizes, and collapses. The canvas takes what remains. An overlay floats over the canvas."
- The header must not grow: "Open, it is two rows. Collapsed, it is one."
  - Tasks 3 to 18 carry the locator band as a third row.
  - Task 18 removes it and meets the rule.
- "Anything hidden comes back by one reliable key or click, in the same place each time."

**Keys**
- "Every key the chrome shows must act."
  - A keycap or key tip is drawn only for a binding where `brightfield_keys::fires(binding.context, dispatch)` holds for the window's current `DispatchContext`.
  - Every new keystroke token gets an arm in `consume_token` (`crates/brightfield-shell/src/window.rs`).
  - Every new key's handler records its verb in slice 1's `MeridianApp::keys_answered`, so slice 1's sweeps in `crates/brightfield-shell/tests/keys_answered.rs` hold it. A new verb added to a menu is held by slice 1's `crates/brightfield-shell/tests/menu_commands.rs`.
  - Every new verb declares slice 1's `label` and `needs`. Keys are rendered with slice 1's `brightfield_keys::keys_label`.
- "Every key binding is a default that an analyst can change in a keymap file, through the key registry." Every key is a `VerbEntry` binding in `crates/brightfield-keys/src/registry.rs`.
- These registry gates stay green:
  - `no_two_verbs_share_a_chord_in_overlapping_contexts`
  - `no_two_verbs_in_one_context_share_a_keystroke_sequence`
  - `no_numeric_binding_addresses_a_surface`, so no modified digit is bound.
- "⌥ is not intercepted while a text field has the keys, since ⌥ types characters there on macOS."

**Commands**
- "The same command list feeds the menu bar, the ribbon, the palette and the keymap."
- "A greyed command that doesn't say why" is an anti-goal. "Greyed means a missing input, never a missing feature."
- The ribbon lists `VerbStatus::Built` verbs only.

**Look**
- "The activity switcher, the open places and the ribbon's tabs must not be mistaken for one another."
- The look comes from `meridian-online/design`: its tokens, square chrome (ADR 0013), Tabler icons (ADR 0009), line tabs, keycaps, and hard shadows on overlays only.
- There are no stand-ins, "since a stand-in tends to stay":
  - `crates/brightfield-shell/tests/token_discipline.rs` stays green with its allowlists unchanged.
  - The shell constructs no `egui::Area`, `egui::Window` or `egui::Modal`.
  - Brightfield's own chrome drawing stays in `crates/brightfield-workbench/src/chrome.rs`.

**Behaviour and platform**
- Staleness keeps today's meaning: "Stale keeps today's meaning … This design adds no detection of age or of changed sources."
- macOS is the validated platform. Linux and Windows reach the same menus in the window.
- Any new `SavedLayout` field carries `#[serde(default)]`, so `LAYOUT_VERSION` is not bumped.

**Copy**
- Use the spec's fixed copy verbatim.
  - Reasons are full sentences in the spec's form, such as "Relate needs two sources."
  - The teaching message reads "next time, press".

**The public repo**
- Keep tracker refs, decision ids, milestone ids and absolute paths out of code, comments, commits and PR text.
- Write the design repo's name as `meridian-online/design`.

**Checks for every task**
- Before each commit, run:
  - `cargo xtask ci --step fmt`
  - `cargo xtask ci --step clippy`
  - the touched tests, with `cargo test -p <crate> --test <file>` or `cargo test -p <crate> <module>::`
- After each commit, run `cargo xtask ci --step comment-citations`.
- Pixel baselines (`crates/brightfield-shell/tests/snapshots/*.png`, which include the gallery's `gallery_<id>_{light,dark}.png`) are drawn only on the Mac Studio:
  - `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test snapshot`
  - `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test gallery_gate every_gated_component_has_light_and_dark_goldens`
- Run one cargo build at a time.

## Review Focus

1. **⌥ typed into a text field.** This covers the title band's Name and Location fields and the palette's query line. No key tips appear, and no tab opens. Task 8 pins it with `option_typed_into_a_text_field_shows_no_key_tips_and_opens_no_tab`.
2. **The window stays movable and zoomable after macOS's title bar is hidden.** A drag on the title band's empty stretch moves the window, and a double-click zooms it. Task 3 pins both.
3. **A narrow window.**
   - The mark, the name, the tabs, the contextual tab and Run never overlap.
   - The activity area gives way first.
   - The command row drops labels to icons rather than clipping, and each command keeps its key through its key tip.
   - Task 6 pins it with `at_the_narrow_window_the_title_band_keeps_run_and_every_tab_whole`.
4. **A layout file from an older build that names retired panes** (`chart-rows`, `chart-controls`, `operation-inspector`). It opens with nothing lost.
   - Task 20 pins it with `a_layout_that_kept_the_grid_in_the_ledger_opens_with_it_on_the_canvas`.
   - Task 23 pins it with `a_layout_saved_with_three_inspector_panes_opens_on_the_one_inspector`.
5. **A run that ends without a record or a stream.** This is a refused or untrusted Protocol, or a runner that would not start. No "running" state is left standing, the activity area says the run failed, and the log says why. Task 4 and Task 5 pin it.

---

## What this plan needs from slice 1

These names come from slice 1, which is built; its plan is removed and its outcome is in the README's "Commands, menus and keys". Every task below uses them as slice 1 defines them. Where the build's names differ from the ones written below, the code's hold: `Condition::ProtocolSource` became `ProtocolToRun` (Run's) and `ProtocolToSave` (Save the spec's), with `ProtocolFolder` for Stop trusting; `menu_rows` takes a `Sheet` (`None`, `Up`, `CloseQuestion`) where it took `sheet_up: bool`; and the test support's `scene_for` takes the binding context after the longname.

**What this plan consumes**

1. **On every `VerbEntry`:** `label: &'static str` and `needs: &'static [Requirement]` (slice 1 Task 3).
   - A label is sentence case, such as "Filter rows", "Sort rows", "Run" or "Organise joins".
   - The ribbon, the activity switcher, the inspector's commands and the teaching message all show it.
2. **Availability, in `brightfield_keys::availability`** (slice 1 Task 3):

   ```rust
   pub enum Need { Sources(u8), Column(ColumnKind), Selected(Selected), Grant(Grant), Condition(Condition) }
   pub struct Requirement { pub need: Need, pub missing: &'static str }
   #[derive(Default)] pub struct Situation { pub sources: usize, pub selected: Vec<Selected>, pub column: Option<ColumnKind>,
                                             pub grants: Vec<Grant>, pub conditions: Vec<Condition> }
   pub enum Availability { Available, Missing(String) }
   pub fn availability(verb: &VerbEntry, situation: &Situation) -> Availability;
   pub fn sentence(longname: &str, need: Need) -> Option<String>;
   ```

3. **In the window** (slice 1 Task 5):
   - `MeridianApp::situation(&self) -> Situation`
   - `MeridianApp::availability_of(&self, longname: &str) -> Option<Availability>`
   - `palette_filter(.., situation: Option<&Situation>)`, with `PaletteCandidate::missing`
   - `CommandPalette::with_situation`
4. **Key labels** (slice 1 Task 1): `KeyPlatform::{Mac, Other}` and `keys_label(keystrokes: &'static str, platform: KeyPlatform) -> String`. For example, `keys_label("cmd-enter", Mac)` is "⌘↩", `"cmd-shift-p"` is "⇧⌘P" and `"w"` is "W".
5. **The menus as data** (slice 1 Task 4), in `brightfield_keys`:
   - `menu_bar() -> Vec<Menu>`, the eight menus Brightfield, File, Edit, View, Protocol, Data, Window and Help;
   - `Menu { title, items: Vec<MenuItem> }`;
   - `MenuItem::{Command { longname, asks }, Separator, Native(NativeItem)}`.

   A task here that adds a window-level command adds it to one of those menus. Slice 1's `every_menu_command_acts_where_it_is_available` then holds it.
6. **Menu commands reach the window through the open queue** (slice 1 Task 8).
   - `OpenQueue::ask_verb` and `take_verbs` carry the commands.
   - A queued verb that `availability_of` finds missing is not applied, and its sentence is said in an Info toast.
   - `MeridianApp::verbs_applied()` lists the verbs `apply` performed on the last frame.
7. **The key sweeps** (slice 1 Tasks 6 and 7):
   - `MeridianApp::keys_answered()`;
   - in the test support `tests/support/scenes.rs`: `scene_for`, `arrange`, `hold_keys`, and `Window::press`.
8. **The mark's menus** (slice 1 Task 10): `MenuHost::{Native, Mark}`, `MeridianApp::mark_rect()` and `mark_menu_drawn()`. On a host with no menu bar, the mark opens the menus.

**What slice 1 leaves to this plan, taken on here**

- **Pressing a greyed command's key shows its sentence.** Task 6 adds `MeridianApp::refuse_key(&mut self, longname: &'static str) -> bool`. Each Global and Workspace key handler of a verb with needs calls it before acting.
  - When `availability_of` says `Missing`, the handler acts on nothing.
  - The sentence shows anchored to the command when it is drawn, or to its tab when the tab is drawn.
  - Otherwise the sentence shows in slice 1's Info toast.
- **Drawing a greyed palette row greyed.** Slice 1 greys a row only through its detail text, because `PickerRow` has no disabled state. The design system adds one (§13 below), and Task 9 draws it.
- **The window's dispatch context.** `Situation` carries none, and the ribbon shows a keycap only where the binding fires. Task 6 adds `MeridianApp::dispatch_context(&self) -> DispatchContext`.
- **A run in progress.** Run's declared needs are `ProtocolSource` and `Trusted`. Task 5 adds `Condition::NotRunning`, with the sentence "The Protocol is running.", appends it to `run-protocol`'s needs, and fills it in `situation()`. It also adds `NotRunning`'s arm to the exhaustive `arrange` in `tests/support/scenes.rs`.
- **The host's key platform.** Task 1 adds `KeyPlatform::host()`, which is `Mac` on macOS and `Other` elsewhere, as slice 1's `MenuHost` default reads.

**Done by slice 1, so not planned here**

- The in-window menus from the mark on Linux and Windows (slice 1 Task 10). They are drawn with egui's own menu widgets, and this plan does not redraw them with the design system's menu row.
- The sweep that every bound key is answered by a handler (slice 1 Tasks 6 and 7).
- Keystroke labels (slice 1 Task 1).

---

## What this plan needs from the design system

A design review in `meridian-online/design` builds these in `meridian-egui`, with pure data in `meridian-design`, which is ADR 0011's crate-boundary test. The interfaces below are this plan's proposals. Each task calls the signature named here. If the review lands a different signature, the consuming task follows the landed one, and its Interfaces block is updated in the same commit.

**What the review landed for slice 2.** The design review closed, and Brightfield pins its result from Task 2 on. Where it differs from the proposals below, the landed form is the one to call:

- **§1 and §3**, `ribbon_tabs` and `key_tip`, landed as proposed. A tab's node is a `Tab` in a `TabList`.
- **§2**, `ribbon_row`, takes groups of `meridian_egui::Button`s rather than a `RibbonCommand` type: `Button::new(label).icon(&icon).keystroke(k).unavailable(reason)`. The row is `spacing::BAND_HEIGHT` (32) tall and each command is a 24 button in it. The design system also carries every icon the taxonomy and the forms name.
- **§4 and §5** are one component, `Popover`. The reason popover is `Popover::show_reason(ctx, id_salt, anchor, reason, keystroke)`, a 240-wide card published as a `Tooltip`. The anchored layer is `Popover::show(ctx, id_salt, anchor, Anchor::Below, body)`, which returns `PopoverResponse { inner, dismissed }`. A menu is `Popover::show_menu` with `menu_item`s. Task 7 calls `Popover::show`, not `egui::Popup`.
- **§9**, the activity area, landed as `run_status(ui, RunStatus)` with `RunStatus::{Running { step, done, total }, Ran { when, age, mark: StepMark }, NotRun { label }}`. `Freshness` is `StepMark::{Ok, Stale, Failed}`, and "not run yet" is the caller's `label`. The component writes the "n of m" count itself. Its node is a `Status` named by its words and the mark's state word.
- **§13**, `PickerRow::unavailable(reason)`, landed as proposed, with one change: Enter or a click on an unavailable row picks nothing, rather than reaching the delegate's confirm, and the reason is already drawn as the row's detail. Task 9 follows the landed behaviour.
- The review also replaced ADR 0011's cap with an inventory, so no entry was composed away for the cap's sake.

**What the review landed for slice 3.** The same design revision carries slice 3's components under general names, and Tasks 12 to 15 call these:

- **§7**, the spine row, is `step_row(ui, StepRow) -> StepRowResponse { response, clicked, fold_clicked }`, with `StepRow::new(StepEntry::{Step, Table}, Gutter, StepMark, label)` and the `output`, `timeline`, `fold`, `focused` and `cursor` builders as proposed. `Gutter` gains `lanes: u8`, the list's widest row, so every row's gutter is one width; a lane at or past it is ignored. The row is `ROW_GRID` (24) tall, and its node is a `Row` named `"<name>, <state word>"`, selected while focused. The fold control is a `Button` named by the row's name.
- **§7's thin form** is `step_mark(ui, StepMark, focused) -> egui::Response`, in a `control::STEP_MARK_CELL` (24) square. Its node carries no role of its own (egui reads it as `Unknown`) and is named by the state word.
- **§7's mark data** landed as proposed, in `meridian_design::marks`: `StepMark`, `STEP_MARKS`, `glyph(mark, dark) -> MarkGlyph { shape: MarkShape, ink, fill }` and `word(mark)`. The state vocabulary stays in the design system.
- **§8**, the open place, is `vertical_tab(ui, VerticalTab) -> VerticalTabResponse { response, clicked, close_clicked }`, with `VerticalTab::new(node, activity).current(..).dirty(..)`. Its node is a `Tab` named `"<node> · <activity>"`.

### Rules the components share

These follow `meridian-egui`'s existing style (`line_tabs`, `list_row`, `key_chip`, `overlay_frame`):

- **Call shape.** A free function `fn widget(ui: &mut egui::Ui, config: Config<'_>) -> WidgetResponse`.
  - `Config` is built with `new(..)` and `#[must_use]` builder methods.
  - The response struct carries `response: egui::Response` and what was clicked.
  - A floating layer is a unit struct with `show(ctx, id_salt, …)`, as `ModalLayer` is.
- **No look parameters.** There is no colour, width, height or radius parameter. The one exception is a bar ink, where `line_tabs` and `list_row` already take one.
- **Heights.** Every height is a ladder rung. Brightfield's gallery gate holds each component's height to a named rung (`crates/brightfield-shell/src/gallery.rs`, item 3).
- **Fit.** Components must fit these slots:
  - the title band, which is `TITLE_BAND_HEIGHT` = `ROW_GRID + 2·SPACE_2`, so its contents draw at the grid rung;
  - the new command-row band, at the same measure;
  - the pane header, which is `header_band_height()` = `control::binding(ROW_GRID).row`.
- **Text comes from Brightfield.** A keystroke is a caller string, already rendered by slice 1's `brightfield_keys::keys_label`, for example `"⌘↩"`, `"W"` or `"⌥S"`. A reason is a caller sentence. A component writes no copy of its own, except where named below.
- **Accessibility.**
  - Every interactive part publishes an accesskit node by role and label.
  - A greyed part is disabled and carries its reason as the node's description, so kittest can find it and a screen reader can say why.
  - Each component reports the rects of its parts, because Brightfield anchors popovers to them and records them as `chrome::NamedControl`s for its tests.
- **Distinct looks.** The ribbon's tabs, the activity switcher and the open places need three treatments that cannot be mistaken for one another. The tokens already carry a segmented fill (`--m-tabs-segmented-bg`) beside the line tabs.
- **Themes.** Light and dark, each with its goldens in the design repo.

**The cap.** ADR 0011 caps `meridian-egui` at 15 primitives, and the crate is near that today. This list adds about a dozen.
- The review either amends the cap, citing this design as the named product requirement, or answers some entries by composition.
- Composition candidates: the key tip from `key_chip`, the teaching message from a label and `key_chip`, the spine's thin form from the spine row, and the anchored layer from egui's own `Popup`, themed by `theme::apply`, as slice 1's in-window menus already use it.
- Where it composes, it names the composition, and the consuming task calls that.
- This is flagged for the owner below.

### 1. A ribbon tab and a contextual tab

**What it draws:**
- One row of stage tabs (Connect, Shape, Model, Publish).
- Then the contextual tab, set apart by a divider and a tint and named for the focused activity and node, for example `Grid · ranking`.
- The open tab is marked.
- While the ribbon is collapsed, the selected tab still reads as selected but not as open.

**Inputs and states:**
- Per tab: a label and an optional keystroke.
- One selected tab or none.
- Collapsed or open.
- Key tips on or off. When on, each tab with a keystroke shows its key tip (§3).
- Tab states: rest, hover, pressed, open, selected-but-collapsed, keyboard focus.
- Clicks and double-clicks are reported per tab.

```rust
pub struct RibbonTab<'a> { /* label, keystroke */ }
impl<'a> RibbonTab<'a> {
    pub fn new(label: &'a str) -> Self;
    pub fn keystroke(self, keystroke: &'a str) -> Self;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPick { Stage(usize), Contextual }
pub struct RibbonTabs<'a> { /* stages, contextual, selected, collapsed, key_tips */ }
impl<'a> RibbonTabs<'a> {
    pub fn new(stages: &'a [RibbonTab<'a>], selected: Option<TabPick>) -> Self;
    pub fn contextual(self, tab: RibbonTab<'a>) -> Self;
    pub fn collapsed(self, collapsed: bool) -> Self;
    pub fn key_tips(self, shown: bool) -> Self;
}
pub struct RibbonTabsResponse {
    pub response: egui::Response,
    pub clicked: Option<TabPick>,        // any tab, the selected one included
    pub double_clicked: Option<TabPick>,
    pub rects: Vec<(TabPick, egui::Rect)>,
}
pub fn ribbon_tabs(ui: &mut egui::Ui, tabs: RibbonTabs<'_>) -> RibbonTabsResponse;
```

Accessibility: one `TabList` holding `Tab`s, with the open one selected.

### 2. A ribbon command, in its row

**What it draws:**
- Each command is an icon, a label and its keycap (`key_chip`'s look).
- Commands are gathered into groups separated by a rule.
- A greyed command keeps its place and its keycap.
- When the row is narrower than its contents, labels are dropped right to left until it fits, and the icon and the key remain.

**Inputs and states:**
- Per command: a Tabler icon, a label, an optional keystroke, and an optional reason, whose presence means unavailable.
- For the row: groups, and key tips on or off.
- Command states: rest, hover, pressed, keyboard focus, unavailable, icon-only, key tip shown.
- An unavailable command senses hover but reports no click.

```rust
pub struct RibbonCommand<'a> { /* icon, label, keystroke, reason */ }
impl<'a> RibbonCommand<'a> {
    pub fn new(icon: &'a Icon, label: &'a str) -> Self;
    pub fn keystroke(self, keystroke: &'a str) -> Self;
    pub fn unavailable(self, reason: &'a str) -> Self;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandAt { pub group: usize, pub index: usize }
pub struct RibbonRow<'a> { /* groups, key_tips */ }
impl<'a> RibbonRow<'a> {
    pub fn new(groups: &'a [&'a [RibbonCommand<'a>]]) -> Self;
    pub fn key_tips(self, shown: bool) -> Self;
}
pub struct RibbonRowResponse {
    pub response: egui::Response,
    pub clicked: Option<CommandAt>,                          // available commands only
    pub pointed_unavailable: Option<(CommandAt, egui::Rect)>,
    pub rects: Vec<(CommandAt, egui::Rect)>,
    pub icon_only: Vec<CommandAt>,
}
pub fn ribbon_row(ui: &mut egui::Ui, row: RibbonRow<'_>) -> RibbonRowResponse;
```

- Accessibility: a `Toolbar`, holding a `Button` per command. An unavailable command is disabled, with its reason as the description.
- Run in the title band is a one-command row.

**Icons the ribbon names.** `meridian-egui`'s icon manifest must carry these glyphs that it does not today:
- for the ribbon's own taxonomy: `adjustments`, `axis-x`, `axis-y`, `palette`, `zoom-in`, `zoom-out`, `arrow-back-up`, `list`, `schema`, `hierarchy`, `link`, `unlink`, `key`, `fold`, `trash` and `layout-columns`;
- for the forms' `FormSpec`s (the operation-forms plan): `eye-off`, `cursor-text`, `math-function`, `transform`, `bucket-droplet`, `copy-off`, `list-numbers`, `sum`, `arrows-join`, `row-insert-bottom` and `file-import`.

### 3. A key tip badge

**What it draws:**
- A small keycap-styled badge over a corner of its target, shown while the caller says key tips are on.
- It is drawn on a layer above the row, so it does not move the target.
- It is non-interactive.

**Inputs:** The target's rect and the keystroke.

```rust
pub fn key_tip(ui: &egui::Ui, target: egui::Rect, keystroke: &str);
```

- `ribbon_tabs` and `ribbon_row` draw it themselves when `key_tips(true)`. It is public so a later surface can take it.
- Accessibility: one `Label` node, labelled with the keystroke.

### 4. A reason popover

**What it draws:**
- A small floating card holding one sentence and, when given, the command's keycap.
- It has the overlay's hard shadow and a 1px rule.
- It is anchored below its anchor, or above when there is no room below.
- It wraps at a width the review sets from the modal-width scale.

**Inputs:** An anchor rect, the sentence, and an optional keystroke. The caller decides when it shows: on hover, or for a few seconds after the command's key is pressed. It holds no state.

```rust
pub struct ReasonPopover;
impl ReasonPopover {
    pub fn show(ctx: &egui::Context, id_salt: impl std::hash::Hash + std::fmt::Debug,
                anchor: egui::Rect, reason: &str, keystroke: Option<&str>) -> egui::Response;
}
```

Accessibility: `Tooltip`.

### 5. An anchored layer

The spec does not list this. It is implied by the collapsed ribbon's dropdown row, because the shell may not construct an `egui::Area`.

**What it draws:** An overlay card (the `overlay_frame` look) anchored below a rect, or above it when there is no room below, holding any body.

**Inputs and states:**
- The anchor and the body.
- It is dismissed by Esc or by a click outside.
- The caller holds whether it is open.

```rust
pub struct AnchoredLayerResponse<R> { pub inner: R, pub dismissed: bool }
pub struct AnchoredLayer;
impl AnchoredLayer {
    pub fn show<R>(ctx: &egui::Context, id_salt: impl std::hash::Hash + std::fmt::Debug,
                   anchor: egui::Rect,
                   body: impl FnOnce(&mut egui::Ui) -> R) -> AnchoredLayerResponse<R>;
}
```

The review may instead rule that egui's own `Popup`, themed by `theme::apply`, is the anchored layer, as slice 1's in-window menus already use it. If so, Task 7 calls `egui::Popup` and this entry is dropped.

### 6. An activity switcher

**What it draws:**
- A small set of mutually exclusive options in a pane header (Grid · Chart, or Model).
- Each option has its keycap.
- It must not read as a tab strip.

**Inputs and states:**
- Per option: a label, an optional keystroke, and an optional reason.
- One selected option.
- Option states: rest, hover, pressed, selected, keyboard focus, unavailable.

```rust
pub struct ActivityOption<'a> { /* label, keystroke, reason */ }
impl<'a> ActivityOption<'a> {
    pub fn new(label: &'a str) -> Self;
    pub fn keystroke(self, keystroke: &'a str) -> Self;
    pub fn unavailable(self, reason: &'a str) -> Self;
}
pub struct ActivitySwitcher<'a> { /* options, selected */ }
impl<'a> ActivitySwitcher<'a> { pub fn new(options: &'a [ActivityOption<'a>], selected: usize) -> Self; }
pub struct ActivitySwitcherResponse {
    pub response: egui::Response,
    pub clicked: Option<usize>,
    pub pointed_unavailable: Option<(usize, egui::Rect)>,
}
pub fn activity_switcher(ui: &mut egui::Ui, switcher: ActivitySwitcher<'_>) -> ActivitySwitcherResponse;
```

Accessibility: a `RadioGroup` holding `RadioButton`s.

### 7. A spine row, with its thin and graph forms

**What it draws (the list form):** One row per step or table, after Flight Explainer:
- the graph's edges in a gutter at the leading edge;
- a state mark;
- the name;
- what it produced (a row count, or a table count);
- for a step only, a timeline bar and a duration;
- for a folded group, a fold control with its member count.

It has two marks of place:
- **focused** is the node the canvas shows, which the activity acts on;
- **cursor** is the row the spine's keys act on, `list_row`'s cursor.

**Inputs and states:**
- The gutter.
- One of five step states: not run, running, ok, failed, stale.
- A label, and whether the row is a step or a table.
- Output text, a timeline as two fractions of the run's span, and duration text.
- A fold, which is none, folded with a count, or unfolded.
- Whether it is focused, and whether it carries the cursor.
- Row states: rest, hover, pressed, focused, cursor, and both at once.

```rust
pub struct Gutter<'a> {
    pub node: Option<u8>,        // the lane this row's node sits on; None for a row with no node
    pub through: &'a [u8],       // lanes passing this row without stopping
    pub from_above: &'a [u8],    // lanes ending at this node (its producers)
    pub to_below: &'a [u8],      // lanes leaving this node (to its consumers)
}
pub use meridian_design::marks::StepMark;   // NotRun, Running, Ok, Failed, Stale
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline { pub start: f32, pub end: f32 }   // 0.0..=1.0 of the run's span
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold { Folded { members: usize }, Unfolded }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpineEntry { Step, Table }
pub struct SpineRow<'a> { /* every field above */ }
impl<'a> SpineRow<'a> {
    pub fn new(entry: SpineEntry, gutter: Gutter<'a>, mark: StepMark, label: &'a str) -> Self;
    pub fn output(self, text: &'a str) -> Self;
    pub fn timeline(self, timeline: Timeline, duration: &'a str) -> Self;
    pub fn fold(self, fold: Fold) -> Self;
    pub fn focused(self, focused: bool) -> Self;
    pub fn cursor(self, cursor: bool) -> Self;
}
pub struct SpineRowResponse { pub response: egui::Response, pub clicked: bool, pub fold_clicked: bool }
pub fn spine_row(ui: &mut egui::Ui, row: SpineRow<'_>) -> SpineRowResponse;
```

**The thin form.** A state mark alone, one per step, in a column, with the focused one marked.

```rust
pub fn spine_mark(ui: &mut egui::Ui, mark: StepMark, focused: bool) -> egui::Response;
```

The cell it allocates is a `meridian-design` constant, `control::SPINE_MARK_CELL`, so `arrangement.rs` can declare the thin rail's width as a `const`.

**The graph form.** The DAG in the rail is drawn by Brightfield's existing Vello asset-graph renderer (`brightfield-render/src/asset_scene.rs`), so that it and the list share one mark.
- The mark's shape and ink per state must be plain data in `meridian-design`: `marks::StepMark` and `marks::glyph(mark: StepMark, dark: bool) -> MarkGlyph`, where `MarkGlyph { shape: MarkShape, ink: Rgba, fill: Option<Rgba> }`.
- `spine_mark` and the Vello renderer both read it.
- The review may keep the state vocabulary app-side, as `status_pill` does, and take `MarkShape × Role` instead. If so, Brightfield maps its five states in `chrome.rs`.

Accessibility: each row is a `Row`, labelled with its name and state word.

### 8. An open place in a vertical list

**What it draws:**
- One row naming a node and an activity, such as `ranking · Grid`.
- An unsaved-draft dot.
- A close control, shown on hover and on the current place.
- The rows stack vertically, the way vertical tabs work in a browser.

**Inputs and states:**
- The node, the activity, whether it is the current place, and whether it is dirty.
- Row states: rest, hover, current, current and hovered, close hovered.

```rust
pub struct OpenPlace<'a> { /* node, activity, current, dirty */ }
impl<'a> OpenPlace<'a> {
    pub fn new(node: &'a str, activity: &'a str) -> Self;
    pub fn current(self, current: bool) -> Self;
    pub fn dirty(self, dirty: bool) -> Self;
}
pub struct OpenPlaceResponse { pub response: egui::Response, pub clicked: bool, pub close_clicked: bool }
pub fn open_place(ui: &mut egui::Ui, place: OpenPlace<'_>) -> OpenPlaceResponse;
```

Accessibility: a `Tab` within a vertical `TabList`.

### 9. The activity area

**What it draws:** It sits in the title band's centre.
- During a run: the step running, how many steps are done of how many, and a progress bar in the progress tokens.
- Otherwise: when the Protocol last ran and how long ago, with a state mark for fresh, stale or failed.
- Before any run: a quiet "not run yet".

**Inputs and states:** The three cases below. Brightfield supplies every word, including the dates.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness { Fresh, Stale, Failed }
pub enum ActivityArea<'a> {
    Running { step: Option<&'a str>, done: usize, total: usize },
    Ran { when: &'a str, age: &'a str, freshness: Freshness },
    NotRun,
}
pub fn activity_area(ui: &mut egui::Ui, area: ActivityArea<'_>) -> egui::Response;   // clickable
```

Accessibility: `Status`, with a `ProgressBar` child while running.

### 10. An inspector section with a title and a count, and a distribution bar

**What the section draws:**
- A header row holding a title, an optional count and a disclosure.
- Under it, the body, drawn only while the section is open.

```rust
pub struct InspectorSection<'a> { /* title, count, open */ }
impl<'a> InspectorSection<'a> {
    pub fn new(title: &'a str, open: bool) -> Self;
    pub fn count(self, count: usize) -> Self;
}
pub struct InspectorSectionResponse<R> { pub response: egui::Response, pub toggled: bool, pub inner: Option<R> }
pub fn inspector_section<R>(ui: &mut egui::Ui, section: InspectorSection<'_>,
                            body: impl FnOnce(&mut egui::Ui) -> R) -> InspectorSectionResponse<R>;
```

**What the distribution bar draws:**
- In chart ink: equal-width bins low to high for a number, or the top values for a category, most frequent first.
- The remainder of the categories as one segment.
- The missing count in `--m-null-ink`.
- The hovered bar reported back, so Brightfield can say its range and count.

```rust
pub enum Bars<'a> { Bins(&'a [u64]), Top { values: &'a [(&'a str, u64)], rest: u64 } }
pub struct DistributionBar<'a> { /* bars, missing, total */ }
impl<'a> DistributionBar<'a> { pub fn new(bars: Bars<'a>, missing: u64, total: u64) -> Self; }
pub struct DistributionResponse { pub response: egui::Response, pub hovered: Option<usize> }
pub fn distribution_bar(ui: &mut egui::Ui, bar: DistributionBar<'_>) -> DistributionResponse;
```

### 11. A menu row with its key

**What it draws:**
- A row with a label and a right-aligned keycap.
- A check, when the row is a toggle.
- A submenu chevron.
- It is greyed when unavailable.

It has two call sites here: the inspector's commands, and the chart encoding's rows. Slice 1's in-window menus, drawn with egui's own widgets, could adopt it later.

In the shell it is named as `meridian_egui::MenuRow`, because slice 1's `menu_bar::MenuRow` is the menus' data.

```rust
pub struct MenuRow<'a> { /* label, keystroke, reason, checked, submenu */ }
impl<'a> MenuRow<'a> {
    pub fn new(label: &'a str) -> Self;
    pub fn keystroke(self, keystroke: &'a str) -> Self;
    pub fn unavailable(self, reason: &'a str) -> Self;
    pub fn checked(self, on: bool) -> Self;
    pub fn submenu(self) -> Self;
}
pub struct MenuRowResponse { pub response: egui::Response, pub clicked: bool, pub pointed_unavailable: Option<egui::Rect> }
pub fn menu_row(ui: &mut egui::Ui, row: MenuRow<'_>) -> MenuRowResponse;
```

Accessibility: a `MenuItem`, disabled with its reason as the description when unavailable.

### 12. The status band's teaching message

**What it draws:** One line, `<command> · next time, press [key]`, with the key as a keycap. It fits the status band's height.
- The connective "next time, press" is the component's own copy, fixed by the spec.
- It appears and goes with no fade, per `speed.md`.

```rust
pub struct TeachingMessage<'a> { /* command, keystroke */ }
impl<'a> TeachingMessage<'a> { pub fn new(command: &'a str, keystroke: &'a str) -> Self; }
pub fn teaching_message(ui: &mut egui::Ui, message: TeachingMessage<'_>) -> egui::Response;
```

Accessibility: `Status`, labelled with the whole sentence.

### 13. A disabled state for a picker row

Slice 1 leaves this to slice 2. Slice 1 lists a palette command whose input is missing with its sentence in the row's detail, but the row is drawn as an available one.

**What it draws:** The row in the disabled ink, with the reason as its detail and its keycap still shown. The row still takes the cursor, so the reason can be read from the keys. Confirming it stays the delegate's, and slice 1's palette answers with the sentence as an error hint.

```rust
// on meridian_egui::PickerRow
pub unavailable: Option<String>,
pub fn unavailable(self, reason: impl Into<String>) -> Self;
```

Accessibility: the row's node is disabled, with the reason as its description.

---

## Decisions this plan makes

The spec leaves these open, under "Open for the plan".

**The locator band does not survive.** The title band, the command row and a locator band would make the open header three rows, and the spec's anti-goal is two. The band's contents move as follows:
- **The Protocol's name and its place**, with the Name and Location fields, rename (F2), move and Choose…, go to the title band in Task 3. The spec puts "the Protocol's name and where it is kept" there.
- **The crumbs** go to the pane header in Task 18, as `step › node`. The first crumb keeps the "Back to every panel" link.
- **The counts** go to the pane header's right end: `table · 19,028 rows`, or `N nodes · M steps` on the graph.
- **The source note** (a panel's or chart spec's name) goes to the title band's document name, keeping `document_source_about` as its hover.

The band stays until Task 18, so that every slice keeps the crumb-back link working.

**The right rail's three tabs fold into one inspector of stacked sections.** The tab strip goes. There is one pane, `inspector`, whose sections are chosen by the selection, most specific first.
1. **Operation**, while a foot field or an operation form is open. This is today's Operation pane.
2. For a column: **Column**, holding what the column is, its values and what can be done with it.
3. For a chart: **Encoding**, **Reads** (the table and the rows drawn), **Fields** (with a count), and **Parameters** (today's sliders).
4. **Step**, which is today's Operator facts for the focused node. It is always last, and collapsed while a section above it is open.

The reason: the spec says the inspector "describes the selection", and a stack of titled, collapsible sections says more of it at once than three tabs, one of which had to be guessed. `open_rail_on_operation` and `restore_rail_after_ask` become "open the Operation section" and "close it".

**The bottom band's tabs map onto the console as follows:**

| Today | Console | Why |
|---|---|---|
| Log | Run log | Same content. During a run it also takes each step's line as the step finishes (Task 27). |
| Quality | Problems | Quality draws only a run header today. Problems lists the last run's failed steps, each with its message, and the quality output when arc reports one. |
| Rows | Retired (Task 20) | Grid is now an activity, and the split shows the grid beside its chart. `move-grid` and its ⌘J go with it. |
| Editor | Editor, a fourth tab, keeping its name | The spec names three tabs. Leaving the spec's YAML editor out removes the only way to edit a chart spec by hand. **The owner accepted this on 2026-11-22.** |
| Versions | Versions | Same content. |

**Other calls, left to the builder by the spec:**

**New verbs, their keys, labels, needs and menus.** All are registry defaults, and none collides in an overlapping context. Each is `CommandTier::View` and `VerbStatus::Built`. Keys are shown as slice 1's `keys_label` renders them.

| Verb | Key (Global) | Label | Needs | Menu | Task |
|---|---|---|---|---|---|
| `show-connect-tab`, `show-shape-tab`, `show-model-tab`, `show-publish-tab` | `alt-c`, `alt-s`, `alt-m`, `alt-p` | Connect tab, Shape tab, Model tab, Publish tab | `Selected::Protocol`, because the front door draws no ribbon | none, since a tab is a motion | 1 |
| `show-context-tab` | `alt-x` | Contextual tab | `Selected::Protocol` | none | 1 |
| `toggle-ribbon` | `alt-r` | Toggle ribbon | `Selected::Protocol` | View | 1, 7 |
| `cycle-spine-width` | `cmd-shift-b` | Spine width | none | Window, after `toggle-outline-rail` | 14 |
| `next-place`, `prev-place`, `close-place` | `cmd-shift-]`, `cmd-shift-[`, `cmd-w` | Next place, Previous place, Close place | `Selected::Protocol` | Window | 15 |
| `quick-open` | `cmd-p` | Quick open | `Selected::Protocol` | File, after `open` (asks) | 16 |
| `show-grid`, `show-chart` | `cmd-alt-g`, `cmd-alt-c` | Grid, Chart | `Selected::Rows`, with the sentences "Grid needs a table or view in focus." and "Chart needs a table or view in focus." | View | 17 |
| `relate` | `cmd-alt-m` | Relate | `Sources(2)` ("Relate needs two sources.") | View | 17 |
| `split-canvas` | `cmd-\` | Split | `Selected::Protocol` | Window | 19 |
| `toggle-console` | `cmd-j`, from Task 26 | Console | `Selected::Protocol` | Window, in `move-grid`'s place | 26 |
| `toggle-inspector-rail` | `cmd-alt-b` | slice 1's label | `Selected::Protocol` | Window | 23 |

The mockup's bare G and C would collide with the chart grammar's `c`. Tasks 15 and 19 add `[`, `]` and `\` to slice 1's `KEY_NAMES`, which `every_registry_binding_parses_to_a_known_key` holds. An activity the node cannot take is greyed with the sentence its verb declares, read through `availability_of`, so the switcher, the ribbon and the menus say one sentence.

**The ribbon's selected tab.** It opens on Shape and stays where the analyst puts it. Switching the canvas changes only the contextual tab's name and commands, because a tab that jumped by itself would be chrome coming and going.

**The ribbon's contents come from two places.**
- **The operation forms' commands come from the forms registry.** The operation-forms plan's `crates/brightfield-shell/src/forms/mod.rs` gives each operation a `FormSpec` with its `stage`, `verb` and `icon`, and `forms::built()` lists the built ones in arc's order. Each tab's first group is the built forms of its stage.
  - A form joins its tab when its `*_BUILT` flag in `crates/brightfield-keys/src/operations.rs` flips, with no edit to the ribbon.
  - Filter and sort are forms there too, with the verbs `filter-rows-at-cursor` and `sort-rows-at-cursor`.
  - The contextual Grid tab lists the built forms asked from the grid (`AsksIn::FootField` and `AsksIn::QuestionLine`), read the same way.
- **Everything else is listed in the ribbon's own taxonomy** (`crates/brightfield-keys/src/ribbon.rs`), which holds no operation verb. These are Built verbs only.
- **Stages, after their forms:**
  - Connect: `open`, `open-sources-browser`.
  - Shape: nothing of its own.
  - Model: `organise-joins`, and `relate` from Task 17.
  - Publish: `copy-chart-picture`, `export-chart-picture`.
- **Contextual tabs:**
  - Grid: nothing of its own, beyond the forms above.
  - Chart: `set-channel`, `put-column-on-x`, `put-column-on-y`, `put-column-on-colour`; `zoom-in`, `zoom-out`, `reset-extent`, `cycle-axis-lock`; `undo`. Slice 1 reserves `change-mark-type`, `add-mark`, `remove-mark` and `cycle-colour-scheme`, so they are not listed.
  - Model: the three model views; `add-join`, `keep-join`, `drop-join`, `change-join-keys`; `rediscover-model`.
  - Graph: `toggle-fold`, `open-steps-sheet`; `yank-address`, `remove-step`.

**One `Stage`.** The forms plan's `forms::Stage` has no Publish, and whichever of the forms registry and Task 1 lands second makes the two one. In this plan, that is `brightfield_keys::ribbon::Stage`, with `forms::Stage` its re-export.

**Run.**
- Run moves from the ledger strip to the title band in Task 5, and the strip's Run action is removed. Two Run controls would be two places to learn one thing.
- While a run is in progress, Run is greyed with "The Protocol is running."

**What the canvas shows before a node is focused.** Today's behaviour stays: the graph at canvas size when the Protocol has no chart. The spec does not say otherwise.

**The thin spine is the navigator's collapsed state.** The `Collapse::To` extent becomes the thin width, so "hidden" there still shows every step's state.

**The status band becomes a band.** It is always drawn, because its left and right ends always have something to say. The ledger's collapsed extent then loses the overlay clearance it carried.

**Timings.** Durations and timeline bars come from arc's live `.jsonl` stream, which Brightfield folds for each step's first `running` timestamp and last terminal timestamp. They show when that file is beside the record, or when arc fills `duration_sec`, and are blank otherwise.

**The teaching message's time.** It lasts 4 seconds, the window's toast length (`meridian_egui::notify::DEFAULT_TOAST_SECONDS`), so the window has one meaning of "a few seconds".

**"Upper quantile".** The plan reads it as the upper quartile, 0.75.

**Persistence.** Only the ribbon's collapsed state is persisted, because the spec asks only for that. The spine's width and the open places are not, and are left out under YAGNI.

## Flagged for the owner

These change what the analyst sees enough for the owner to rule on them. **The owner ruled on the first three on 2026-11-22, accepting each:** the console holds Run log, Problems, Versions and Editor; the grid has two instances, one beside a chart in the split and one as the Grid activity on a table or view an operation produced; and the graph's key hints move to the status band. Number 4 is with arcform as a request for per-step timing in the run record, and number 5 is with the design review.

1. **Rows leaves the bottom band, and ⌘J changes job** (Tasks 20 and 26).
   - The grid under a chart becomes the split, ⌘\ (Task 19).
   - ⌘J stops moving the grid and opens and closes the console.
2. **The chart spec's YAML editor becomes a fourth console tab, keeping its name "Editor"** (Task 26).
   - The spec lists three tabs.
   - The alternative is an Editor activity on chart nodes, which would reopen "Grid and Chart are the activities".
3. **The hint band is retired** (Task 28).
   - The graph's key grammar, today a band of its own under the canvas, becomes the status band's centre while the graph holds the canvas, except while a teaching message shows.
   - The spec's chrome does not mention the band.
4. **Stale marks and timeline bars will seldom show.**
   - Nothing live feeds the staleness overlay today (`EditOverlay` is fed only by unit tests). The spec rules out new detection.
   - Per-step durations survive only while arc's stream file does, because arc's `duration_sec` is "deferred (null)".
   - An arcform change that records per-step start and finish would make the spine's timings permanent. That would be a cross-repo ruling.
5. **The design system's primitive cap.** ADR 0011 caps `meridian-egui` at 15 primitives, and this list roughly doubles the crate. The design review has to amend the cap or compose, and that is a ruling in the design repo.

## Gaps in the spec

- **No per-step timings.** The run record carries no per-step timing (`duration_sec` is deferred). Shipping the spine row's timeline bar and duration needs the stream fold in Task 11, or an arcform change.
- **"Today's meaning" of stale is not computed live.** As written, the activity area's "stale" and the spine's stale mark never appear.
- **"Upper quantile" is undefined.** This plan reads it as 0.75.
- **The session holds one table.** A Grid or Chart of any other table draws "No rows for …" today. Quick open and the switcher will land there until the engine holds more than one table. That work is not in this design.
- **Section 9's palette grouping by stage belongs to no slice.** This plan puts it in slice 2 (Task 10).
- **An anchored layer is missing from the component list.** It is needed for the collapsed ribbon's dropdown row, because the shell may not build overlays. It is added above as §5.
- **A disabled picker row is missing from the component list.** Slice 1 greys palette rows only in their text. It is added above as §13.
- **The spec is silent on today's hint band and Editor pane.** This plan decides both, and flags both.

---

## File structure

**`crates/brightfield-keys/`**

| Path | Change | Responsibility |
|---|---|---|
| `src/ribbon.rs` | Create | `Stage`, `Context`, the stage and contextual command groups, each command's Tabler icon name |
| `src/keystroke.rs` | Modify (slice 1's) | `KeyPlatform::host()` |
| `src/availability.rs` | Modify (slice 1's) | `Condition::NotRunning` |
| `src/menus.rs` | Modify (slice 1's) | The new window-level commands join their menus; `move-grid` leaves Window |
| `src/registry.rs` | Modify | The new verbs, with their keys, labels and needs; retire `move-grid`; console verbs become Built |
| `src/lib.rs` | Modify | Re-exports |

**`crates/brightfield-protocol/`**

| Path | Change | Responsibility |
|---|---|---|
| `src/stream.rs` | Modify | `StepTimes`, `step_times` |

**`crates/brightfield-engine/`**

| Path | Change | Responsibility |
|---|---|---|
| `src/profile.rs`, `src/lib.rs` | Modify | The upper quartile; profiling a table on demand |

**`crates/brightfield-workbench/`**

| Path | Change | Responsibility |
|---|---|---|
| `src/arrangement.rs` | Modify | `RIBBON_BAND`, `Collapse::Away`, the thin and graph widths; remove `LOCATOR_BAND` and `HINT_BAND`; the status band becomes a band; the inspector's and console's pane lists |
| `src/chrome.rs` | Modify | The pane header's crumb, switcher and size note; the status band's centre slot; the strip's keycap |
| `src/subject.rs` | Modify | `Subject.activities`, `Subject.size_note` |
| `src/item.rs` | Modify | `Request::VerbByPointer` |
| `src/persist.rs` | Modify | `SavedLayout.ribbon_collapsed`; an old `GridSpot::Ledger` parses as `Canvas` |

**`crates/brightfield-shell/src/`**

| Path | Change | Responsibility |
|---|---|---|
| `ribbon.rs` | Create | `RibbonState`, `CommandView`, `ribbon_views`; draws the tabs, the command row, the dropdown and the reason popover |
| `title_band.rs` | Create | The unified title band: controls inset, drag and zoom, mark, name and place, activity area, Run |
| `spine.rs` | Create | `SpineState`, `Lanes`, `lanes`, `timelines`; the three widths; open places |
| `node_activity.rs` | Create | `NodeActivity`, `ActivityOffer`, `offers`; Chart opens, asks or drafts |
| `run.rs` | Modify | `live_stream`, `RunProgress`, `progress`, `describe_ran` |
| `protocol.rs` | Modify | `SpineRow`'s new fields; `charts_reading`; `NodeView::Chart`; the console panes' titles and content |
| `inspector.rs` | Modify | Becomes the one inspector, with its sections |
| `overlays.rs` | Modify | The palette grouped by stage; `QuickOpen`; `ChooseChart` |
| `window.rs` | Modify | The draw path for each region; `consume_token` arms; dispatch; window sizing (`chrome_budget`, `boot_tile_body`) |
| `gallery.rs` | Modify | One `Component` demo per adopted primitive |
| `menu_bar.rs` (slice 1's) | Modify | A menu choice is a pointer use, for the teaching message (Task 29) |
| `forms/` (the forms plan's) | Read only | `forms::built()`, `by_verb` and `FormSpec` feed the ribbon, the palette's stages and the inspector's column commands |
| `main.rs` | Modify | The macOS viewport flags |

**`crates/brightfield-shell/tests/`**

| Path | Change | Responsibility |
|---|---|---|
| `title_band.rs`, `ribbon.rs`, `spine_widths.rs`, `open_places.rs`, `quick_open.rs`, `activity_switcher.rs`, `console.rs`, `teaching.rs` | Create | Window-level tests, each with its own `Live` harness (`MeridianApp::headless`, `ctx.run_ui(raw, \|ui\| app.draw(ui))`), as `navigator_spine.rs` builds one |
| `support/scenes.rs` (slice 1's) | Modify | `arrange` gains `Condition::NotRunning`'s arm |
| `arrangement.rs`, `navigator_spine.rs`, `canvas_pane_group.rs`, `inspector_contract.rs`, `operation_panel.rs`, `status_rail.rs`, `versions_panel.rs`, `run_control.rs`, `rename_protocol.rs`, `protocol_panels_window.rs`, `gallery_gate.rs` | Modify | Re-pinned where a region moves |

**Repo root**

| Path | Change | Responsibility |
|---|---|---|
| `README.md` | Modify | The fold, in Task 30 |

---

## Slice 2: the unified title band and the ribbon, with key tips, collapse and greyed reasons

The slice is finished when:
- a Protocol window draws the title band and the command row;
- every visible command shows its key, and a greyed one says why;
- ⌥ shows key tips;
- the ribbon collapses and remembers it;
- a greyed command's key says its sentence;
- the palette draws a greyed row greyed.

Linux and Windows already reach the menus from the mark (slice 1 Task 10). The locator band stays, carrying the crumbs, the counts and the source.

### Task 1: The ribbon's taxonomy and its keys

**Blocked by:** slice 1 merged (`VerbEntry::label`, `needs`, `keys_label`, `KeyPlatform`, `menu_bar`, `keys_answered`). Nothing from the design system.

**Files:**
- Create: `crates/brightfield-keys/src/ribbon.rs`, `crates/brightfield-shell/src/ribbon.rs`
- Modify:
  - `crates/brightfield-keys/src/registry.rs`
  - `crates/brightfield-keys/src/keystroke.rs` (`KeyPlatform::host`)
  - `crates/brightfield-keys/src/menus.rs` (`toggle-ribbon` in View)
  - `crates/brightfield-keys/src/lib.rs`
  - `crates/brightfield-shell/src/window.rs` (`consume_token`; the tab verbs' handlers)
- Test: unit tests in `ribbon.rs`; slice 1's `crates/brightfield-shell/tests/keys_answered.rs` and `menu_commands.rs`

**Interfaces:**
- Consumes: slice 1's `VerbEntry::{label, needs}`, `Requirement`, `KeyPlatform`, `menu_bar`, `keys_answered`; `registry()`; `fires`.
- Produces:
  - `ribbon::{Stage, Context, Group, RibbonEntry, stage_groups, context_groups, stage_of}`, where:
    - `Stage::{Connect, Shape, Model, Publish}`, with `Stage::ALL`, `label()` and `tab_verb()`;
    - `Context::{Grid, Chart, Model, Graph}`, with `label()`;
    - `pub struct RibbonEntry { pub longname: &'static str, pub icon: &'static str }`;
    - `pub type Group = &'static [RibbonEntry]`;
    - `stage_groups(Stage) -> &'static [Group]` and `context_groups(Context) -> &'static [Group]`;
    - `stage_of(&str) -> Option<Stage>`, for the taxonomy's own verbs only. A form's stage is its `FormSpec`'s.
  - `impl KeyPlatform { pub const fn host() -> Self }`: `Mac` on macOS and `Other` elsewhere.
  - The six verbs `show-connect-tab`, `show-shape-tab`, `show-model-tab`, `show-publish-tab`, `show-context-tab` and `toggle-ribbon`, with the keys, labels, needs and menu from "Decisions". Each is Global, `CommandTier::View`, `Drives::PaletteMeta`, `VerbStatus::Built`, and scored, with the motor note "⌥ and the tab's initial, the Office KeyTip move". Each applies at Dashboard, View, Protocol and Model.
  - In `crates/brightfield-shell/src/ribbon.rs`: `pub enum RibbonPick { Stage(Stage), Context }` and `pub struct RibbonState { pub selected: RibbonPick, pub collapsed: bool }`, so the tab verbs have something to act on. Task 6 draws it, and Tasks 6 and 7 add its other fields.

- [ ] **Step 1: Write the failing tests.**

  ```rust
  // ribbon.rs
  #[test] fn the_ribbon_lists_only_built_commands() { /* every RibbonEntry.longname is in registry() with VerbStatus::Built */ }
  #[test] fn joining_sits_under_model() { assert_eq!(stage_of("organise-joins"), Some(Stage::Model)); }
  #[test] fn the_ribbons_own_lists_hold_no_operation_verb() { /* no entry satisfies operations::is_operation_verb, or is filter-rows-at-cursor or sort-rows-at-cursor */ }
  #[test] fn every_tab_has_a_bound_key_of_its_own() { /* each Stage::tab_verb() and "show-context-tab" is bound, once */ }
  #[test] fn the_host_platform_is_the_mac_on_macos() { assert_eq!(KeyPlatform::host() == KeyPlatform::Mac, cfg!(target_os = "macos")); }
  ```

  Slice 1's sweeps then fail until the handlers record, with the six new verbs: `every_global_key_is_answered`, and `every_menu_command_acts_where_it_is_available` for `toggle-ribbon`. The icon names are checked in Task 2, once the pin carries the new glyphs.

- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-keys ribbon::` and `cargo test -p brightfield-shell --test keys_answered --test menu_commands`.

- [ ] **Step 3: Implement.**
  - Write the starting contents from "Decisions" as `static` groups.
  - The taxonomy holds no operation verb: neither the forms' verbs nor `filter-rows-at-cursor` and `sort-rows-at-cursor`.
  - Use the icon names from the design section's list for the ribbon's own taxonomy: `folder`, `database`, `hierarchy` (`organise-joins`), `copy`, `download`, `adjustments`, `axis-x`, `axis-y`, `palette`, `zoom-in`, `zoom-out`, `maximize`, `lock`, `arrow-back-up`, `chart-dots`, `list`, `file`, `link`, `check`, `unlink`, `key`, `refresh`, `fold`, `trash`.
  - If the forms plan's Task 2 has landed, replace `forms::Stage` with a re-export of `ribbon::Stage`, which adds Publish. The forms plan's `each_built_form_is_filed_under_its_stage` stays green.
  - Add the six verbs, each with its `label` and `needs`.
  - Add `toggle-ribbon` to slice 1's View menu.
  - Add `consume_token` arms for `alt-c`, `alt-s`, `alt-m`, `alt-p`, `alt-x` and `alt-r`, as `consume_key(Modifiers::ALT, Key::_)`.
  - Add handlers that set `RibbonState.selected` (or toggle `collapsed`, a no-op until Task 7) and record the verb in `keys_answered`.
  - Add `apply` arms for the same verbs, so the queue and the palette reach them, recorded in `verbs_applied`.

- [ ] **Step 4: Run the tests, and expect them to pass.** Also run `cargo test -p brightfield-keys`, since slice 1's label, sentence and collision gates must stay green.

- [ ] **Step 5: Commit.** Message: "The ribbon's stages, its tabs' keys and where they are declared".

### Task 2: Adopt slice 2's components

**Blocked by:** `ribbon_tabs`, `ribbon_row`, `key_tip`, `ReasonPopover`, `AnchoredLayer` (unless the review rules for `egui::Popup`), `activity_area` and `PickerRow`'s disabled state landed in `meridian-online/design`, with the new icons.

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`, the `rev` on `meridian-design` and `meridian-egui`), `Cargo.lock`, `crates/brightfield-shell/src/gallery.rs`
- Create: `crates/brightfield-shell/tests/ribbon.rs`
- Test: `crates/brightfield-shell/tests/gallery_gate.rs`

**Interfaces:**
- Consumes: the design section's §1–5, §9 and §13.
- Produces:
  - Gallery entries `ribbon-tabs`, `ribbon-row`, `key-tip`, `reason-popover`, `popover` and `run-status`, each a `Component` with a `ComponentInfo`, named for the components the review landed.
  - The gallery's probe roles gain `Tab`, `Status` and `Tooltip`.
  - The existing `PickerDemo` shows a disabled row once Task 9 greys the palette's rows. Its delegate is the shipping `CommandPalette`, which does not mark a row unavailable until then.

- [ ] **Step 1: Bump the pin and add the demos.** Bump both revs to the design commit that carries the components. Add one `impl Component` demo per primitive to `catalog()`, each `ComponentStatus::Draft`, with a probe by role and label. A demo drawn in the flow (the tabs, the row and the run status) declares a `GateHeight::Rung`; a floating one (the key tip and both popovers) declares `GateHeight::Intrinsic` with its reason, since its card sizes to what it holds.
- [ ] **Step 2: Check the ribbon's icons.** Add `every_ribbon_command_names_an_icon_the_design_system_carries` to `crates/brightfield-shell/tests/ribbon.rs`, asserting `meridian_egui::Icon::by_name(entry.icon).is_some()` for every `RibbonEntry` of the taxonomy's stages and contexts. The forms' icons are checked in Task 6.
- [ ] **Step 3: Run the tests.** Run `cargo test -p brightfield-shell --test gallery_gate --test ribbon`. Expect `the_catalog_is_complete_against_the_source` and the icon test to pass.
- [ ] **Step 4: Draw the goldens on the Mac Studio.** Promote each entry to `ComponentStatus::Ready`. Draw its goldens with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test gallery_gate every_gated_component_has_light_and_dark_goldens`. Then rerun without `UPDATE_SNAPSHOTS`, and expect a pass.
- [ ] **Step 5: Commit.** Message: "Take the ribbon's components from the design system". Include the new `tests/snapshots/gallery_*` files.

### Task 3: The unified title band, with the Protocol's name and place

**Blocked by:** nothing from the design system.

**Files:**
- Create: `crates/brightfield-shell/src/title_band.rs`, `crates/brightfield-shell/tests/title_band.rs`
- Modify: `crates/brightfield-shell/src/main.rs` (the viewport), `crates/brightfield-shell/src/window.rs` (`title_band`, `place_band_ui`'s call site, `locator_band_ui`), `crates/brightfield-workbench/src/arrangement.rs` (`WINDOW_CONTROLS_INSET`)
- Test: `title_band.rs`; update `rename_protocol.rs` (`rename_is_kept_at_once`, `the_location_field_moves_it`) and `protocol_panels_window.rs`

**Interfaces:**
- Produces:
  - `pub const WINDOW_CONTROLS_INSET: f32`, a sum of `meridian_design::spacing` constants. It is used on macOS only, and is 0.0 elsewhere through `title_band::controls_inset() -> f32`, which reads `cfg!(target_os = "macos")`.
  - `MeridianApp::title_place_rects()`, which replaces `locator_name_rect`, `locator_location_rect` and `locator_choose_rect` under the same meanings.

- [ ] **Step 1: Write the failing tests.**
  - `the_title_band_leaves_the_window_controls_their_corner`: gated `#[cfg(target_os = "macos")]`. The home mark's rect has `left() >= WINDOW_CONTROLS_INSET`.
  - `a_drag_on_the_title_bands_empty_stretch_moves_the_window`: press and move on the band between the name and the band's right end. The frame's `FullOutput.viewport_output[&ViewportId::ROOT].commands` contains `ViewportCommand::StartDrag`.
  - `a_double_click_on_the_title_bands_empty_stretch_zooms_the_window`: the commands contain `ViewportCommand::Maximized(true)`.
  - `the_protocols_name_and_place_are_kept_in_the_title_band`: `title_place_rects()` lie inside `region_rect(TITLE_BAND)`.
  - `the_locator_band_keeps_the_crumbs_and_counts`: `locator_crumbs()` is unchanged.
  - `rename_is_kept_at_once` still passes when F2 opens the Name field in its new place.
  - Slice 1's `crates/brightfield-shell/tests/mark_menu.rs` stays green, because the mark keeps slice 1's `MenuHost` behaviour: home on the Mac host, the eight menus on the Mark host, drawn on the front door too.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test title_band`.
- [ ] **Step 3: Implement.**
  - On macOS, set `.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)` on the `ViewportBuilder` in `main.rs`. Keep `with_title`, which the Window menu and Mission Control read.
  - Move `place_band_ui`'s call into `title_band`, after the name.
  - Make the band's own leftover rect sense drag and double-click, sending `StartDrag` and `Maximized(!maximized)`.
  - The mark keeps slice 1's `MenuHost` behaviour and its `mark_rect()`.
- [ ] **Step 4: Run the tests, and expect them to pass.** Run `cargo test -p brightfield-shell --test title_band --test rename_protocol --test protocol_panels_window --test arrangement --test mark_menu`.
- [ ] **Step 5: Verify by eye on the Mac Studio.** Run the app. The traffic lights sit clear of the mark, the band drags the window, and a double-click zooms it.
- [ ] **Step 6: Commit.** Message: "The title band takes the window's controls, the Protocol's name and where it is kept".

### Task 4: Run progress, read from arc's live stream

**Blocked by:** nothing from the design system.

**Files:**
- Modify: `crates/brightfield-shell/src/run.rs`, `crates/brightfield-shell/src/window.rs` (`PendingRun`, the per-frame poll)
- Test: unit tests in `run.rs`; `crates/brightfield-shell/tests/run_control.rs`

**Interfaces:**
- Consumes: `brightfield_protocol::stream::{StreamReader, StreamState}`; `run::runs_dir`; `RunView.finished_at`; `brightfield_workbench::RunState`.
- Produces:
  - `run::live_stream(dir: &Path, since: SystemTime) -> Option<PathBuf>`: the newest `*.jsonl` in `runs_dir(dir)` modified at or after `since`.
  - `pub struct RunProgress { pub running: Option<String>, pub done: usize, pub total: usize }`.
  - `run::progress(state: &StreamState, total: usize) -> RunProgress`.
  - `pub struct LastRun { pub when: String, pub age: String, pub state: RunState }`.
  - `run::describe_ran(finished_at: &str, now: chrono::DateTime<chrono::Local>) -> Option<(String, String)>`.
  - `MeridianApp::run_progress(&self) -> Option<RunProgress>` and `MeridianApp::last_run(&self) -> Option<LastRun>`.

- [ ] **Step 1: Write the failing tests.**

  ```rust
  #[test] fn progress_counts_the_steps_done_and_names_the_one_running() {
      let s = fold_stream("{\"step\":\"fetch\",\"state\":\"success\",\"ts\":\"t\"}\n{\"step\":\"rank\",\"state\":\"running\",\"ts\":\"t\"}\n");
      assert_eq!(progress(&s, 3), RunProgress { running: Some("rank".into()), done: 1, total: 3 });
  }
  #[test] fn a_run_is_described_by_its_day_and_its_age() {
      let now = Local.with_ymd_and_hms(2026, 11, 21, 10, 0, 0).unwrap();
      assert_eq!(describe_ran("2026-10-23T09:14:00+00:00", now).map(|(w, _)| w), Some("ran 23 Oct".into()));
      assert_eq!(describe_ran("2026-10-23T09:14:00+00:00", now).map(|(_, a)| a), Some("29 days ago".into()));
  }
  #[test] fn the_live_stream_is_the_newest_jsonl_written_since_the_run_began() { /* temp dir: an old .jsonl, then a new one */ }
  ```

  In `run_control.rs`, add `a_run_refused_before_its_step_loop_leaves_no_progress_standing`: with an untrusted Protocol, after the run is taken, `run_progress()` is `None`, and `last_run()`'s state is `RunState::Failed`.

  Age words:
  - under a minute: "just now";
  - under an hour: "N minutes ago";
  - under a day: "N hours ago";
  - otherwise: "N days ago", with "1 day ago" singular.

  The day reads "ran 23 Oct" in the same year, and "ran 23 Oct 2025" otherwise, in local time.

- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell run::` and `cargo test -p brightfield-shell --test run_control`.
- [ ] **Step 3: Implement.**
  - `PendingRun` gains `stream: Option<StreamReader>` and `since: SystemTime`.
  - Each frame of a run, find the stream once with `live_stream`, then `poll` it.
  - `total` is the Protocol's step count, the number `spine_caption` reports.
  - `last_run` reads `model.run`'s `finished_at` and outcome. Staleness reads the existing `RunState`.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "Follow a run's progress on arc's live stream, and say when the Protocol last ran".

### Task 5: The activity area and Run, in the title band

**Blocked by:** `activity_area` and `ribbon_row` (Task 2). It also needs Tasks 1 and 4.

**Files:**
- Modify:
  - `crates/brightfield-shell/src/title_band.rs`
  - `crates/brightfield-shell/src/window.rs`: remove `run_action` from the ledger strip's `Trailing`; `situation()` fills `NotRunning`.
  - `crates/brightfield-keys/src/availability.rs` (`Condition::NotRunning`)
  - `crates/brightfield-keys/src/registry.rs` (`run-protocol`'s needs)
  - `crates/brightfield-shell/tests/support/scenes.rs` (`arrange`'s `NotRunning` arm)
- Test: `crates/brightfield-shell/tests/title_band.rs`, the unit tests in `availability.rs`; update `crates/brightfield-shell/tests/run_control.rs`

**Interfaces:**
- Consumes:
  - `meridian_egui::{activity_area, ActivityArea, Freshness, ribbon_row, RibbonRow, RibbonCommand, icons::PLAYER_PLAY}`
  - `MeridianApp::{run_progress, last_run}`
  - slice 1's `availability_of`, `Situation`, `keys_label`, and `KeyPlatform::host` (Task 1)
- Produces:
  - `Condition::NotRunning`, met while no run is in progress. It is appended to `run-protocol`'s needs as `Requirement::new(Need::Condition(Condition::NotRunning), "The Protocol is running.")`.
  - A `NamedControl` "Run", and one for the activity area. A click on the activity area opens the ledger rail on Log.

- [ ] **Step 1: Write the failing tests.**
  - In `availability.rs`: `run_waits_for_the_run_in_progress`. A situation holding `ProtocolSource` and `Trusted` but not `NotRunning` gives `Missing("The Protocol is running.")`. Slice 1's `meeting_meets_every_listed_need` stays green.
  - `the_title_band_offers_run_with_its_key`: a `NamedControl` named "Run" lies in `TITLE_BAND`, and the band's text includes `keys_label("cmd-enter", KeyPlatform::host())`.
  - `while_a_run_is_in_progress_run_is_greyed_and_says_why`: a pointer hover shows "The Protocol is running."
  - `on_a_chart_start_run_is_greyed_with_its_declared_sentence`: "Run needs a Protocol of your own: open a data file or a folder."
  - `the_activity_area_names_the_step_running_and_how_many_are_done`
  - `after_a_run_the_activity_area_says_when_and_how_long_ago`
  - `a_failed_run_shows_failed_in_the_activity_area`
  - In `run_control.rs`, `the_housing_file_offers_a_run_control_on_the_ledger_strip` becomes `the_housing_file_offers_run_in_the_title_band`, and `a_protocol_with_no_spec_behind_it_draws_no_run_control` keeps its meaning.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-keys availability::` and `cargo test -p brightfield-shell --test title_band --test run_control --test keys_answered --test menu_commands`.
- [ ] **Step 3: Implement.**
  - Run is a one-command `ribbon_row`. Its reason is `availability_of("run-protocol")`'s `Missing` sentence.
  - Map the window's `RunState` to `Freshness`: Fresh and Previewed become Fresh, StaleOwnEdit and StaleUpstream become Stale, and Failed becomes Failed. `NeverRun` becomes `ActivityArea::NotRun`.
  - Lay out the band: the left group (inset, mark, name and place), then the stage tabs (Task 6), the activity area centred in what is left, and Run at the right end.
  - When the band is too narrow, drop the activity area first.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "Run and what the Protocol is doing sit in the title band".

### Task 6: The ribbon's tabs, its command row and greyed reasons, by pointer and by key

**Blocked by:** `ribbon_tabs`, `ribbon_row` and `ReasonPopover` (Task 2), slice 1's `situation` and `availability_of`, and the operation-forms plan's phase 1 merged (its Tasks 1–6: the forms registry, with filter and sort as forms). It also needs Tasks 1, 3 and 5.

**Files:**
- Modify:
  - `crates/brightfield-shell/src/ribbon.rs`
  - `crates/brightfield-workbench/src/arrangement.rs`
  - `crates/brightfield-shell/src/window.rs`: the draw path; `chrome_budget`; `boot_tile_body`; `dispatch_context`; `refuse_key` in each Global and Workspace key handler of a verb with needs.
- Test: `crates/brightfield-shell/tests/ribbon.rs`, `crates/brightfield-shell/tests/arrangement.rs`, the unit tests in `arrangement.rs`

**Interfaces:**
- Consumes:
  - Task 1's taxonomy and `KeyPlatform::host`
  - slice 1's `Situation`, `availability`, `availability_of`, `keys_label`, and its Info toast for a refused verb
  - the forms plan's `forms::{built, FormSpec, AsksIn}`
  - the design section's §1, §2 and §4
- Produces:
  - In `arrangement.rs`:
    - `pub const RIBBON_BAND: RegionId = RegionId::new("ribbon-band")`.
    - `pub const RIBBON_BAND_HEIGHT: f32 = TITLE_BAND_HEIGHT`.
    - A `Collapse::Away` variant, meaning the user may put the band away entirely. It is allowed on bands only. `Collapse::extent()` gives `Some(0.0)` for it.
    - `audit_arrangement`'s positive-extent, below-default and is-a-rail findings apply to `To` only. A new finding refuses `Away` on anything that is not an `Extent::Band`.
    - The region `{ RIBBON_BAND, Edge::Top, Extent::Band(RIBBON_BAND_HEIGHT), Occupant::Chrome("the selected tab's commands"), Collapse::Away, RegionFrame::Band }`, declared after `TITLE_BAND`.
  - In `ribbon.rs`:
    - `RibbonState` gains `dropdown: bool` and `reason: Option<ReasonShown>`.
    - `pub struct ReasonShown { pub longname: &'static str, pub sentence: String, pub until: Instant }`.
    - `pub struct CommandView { pub longname: &'static str, pub label: &'static str, pub icon: &'static str, pub keycap: Option<String>, pub unavailable: Option<String> }`.
    - `pub fn stage_commands(stage: Stage) -> Vec<Vec<RibbonEntry>>`: first a group of the built forms of that stage, in `forms::built()`'s order, as `RibbonEntry { longname: spec.verb, icon: spec.icon }`; then `ribbon::stage_groups(stage)`. An empty group is dropped.
    - `pub fn context_commands(context: Context) -> Vec<Vec<RibbonEntry>>`: for `Grid`, first a group of the built forms asked from the grid (`AsksIn::FootField(_) | AsksIn::QuestionLine`); then `ribbon::context_groups(context)`.
    - `pub fn ribbon_views(groups: &[Vec<RibbonEntry>], situation: &Situation, dispatch: DispatchContext, platform: KeyPlatform) -> Vec<Vec<CommandView>>`. It is pure: the keycap is `keys_label` of the verb's first binding that `fires` in `dispatch`, or `None`. `unavailable` is `availability`'s `Missing` sentence.
  - In the window:
    - `MeridianApp::dispatch_context(&self) -> DispatchContext`, from what holds the keys. This is the context slice 1's `Situation` leaves out.
    - `MeridianApp::refuse_key(&mut self, longname: &'static str) -> bool`. It is true when `availability_of(longname)` is `Missing`, and it then shows the sentence: anchored to the command when the command is drawn, else to its stage tab when that is drawn, else in slice 1's Info toast. A handler that is refused acts on nothing and records nothing in `keys_answered`.
    - `MeridianApp::ribbon_context() -> Option<(Context, String)>`, read from `CanvasHolds` and the model pane. Graph gives `Graph`, a View on Grid gives `Grid`, Dashboard and Chart give `Chart`, and the model pane gives `Model`. The `String` is the node's label.

- [ ] **Step 1: Write the failing tests.**

  ```rust
  #[test] fn the_shape_tab_leads_with_the_built_forms_in_arcs_order() {
      let verbs: Vec<_> = forms::built().filter(|f| f.stage == Stage::Shape).map(|f| f.verb).collect();
      assert_eq!(stage_commands(Stage::Shape)[0].iter().map(|e| e.longname).collect::<Vec<_>>(), verbs);
      assert_eq!(verbs[..2], ["filter-rows-at-cursor", "sort-rows-at-cursor"]);
  }
  #[test] fn a_command_whose_key_does_not_fire_here_shows_no_keycap() {
      let views = ribbon_views(&stage_commands(Stage::Shape), &Situation::default(),
                               DispatchContext::CanvasFocused, KeyPlatform::Mac);
      assert_eq!(views[0][0].keycap, None);           // `w` binds in the Grid context only
  }
  #[test] fn a_greyed_command_carries_the_sentence_its_needs_declare() {
      let views = ribbon_views(&stage_commands(Stage::Shape), &Situation::default(),
                               DispatchContext::GridFocused, KeyPlatform::Mac);
      assert_eq!(views[0][0].keycap.as_deref(), Some("W"));
      assert_eq!(views[0][0].unavailable.as_deref(), Some("Filter rows needs the grid's cursor on a cell."));
  }
  ```

  Also in `ribbon.rs`: `every_built_form_names_an_icon_the_design_system_carries`, over `forms::built()`. A form task that flips its `*_BUILT` meets this test, so the design system must carry the form's glyph before the form reaches the ribbon.

  Window-level tests in `ribbon.rs`:
  - `the_command_row_draws_under_the_title_band_at_its_declared_height`
  - `clicking_a_stage_tab_shows_its_commands`
  - `the_contextual_tab_is_named_for_the_activity_and_the_node`: "Grid · ranking" on a grid.
  - `pointing_at_a_greyed_command_opens_its_reason`
  - `pressing_a_greyed_commands_key_shows_the_same_sentence`: ⌘↩ on a chart start shows Run's declared sentence anchored to Run, and nothing runs.
  - `a_greyed_commands_key_still_says_why_when_its_tab_is_not_open`: the popover is anchored to its stage tab.
  - `every_global_key_of_a_greyed_command_says_its_sentence`: over slice 1's `door()` scene, for each built verb with a Global binding whose `availability_of` is `Missing`, `Window::press` leaves its sentence on screen and leaves the verb out of `keys_answered()`.
  - `at_the_narrow_window_the_title_band_keeps_run_and_every_tab_whole`: at 900×600 logical, no two of the mark, name, tabs, contextual tab and Run intersect, and Run lies inside the band.

  In `arrangement.rs`:
  - The unit test `the_audit_refuses_away_on_anything_but_a_band`.
  - `the_shipped_arrangement_lets_a_rail_collapse_and_a_band_not` becomes "rails collapse To, the ribbon band goes Away, other bands are Fixed".
  - `the_drawn_regions_match_the_declared_arrangement` stays green.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-workbench arrangement::` and `cargo test -p brightfield-shell --test ribbon --test arrangement --test keys_answered`.
- [ ] **Step 3: Implement.**
  - Draw the tabs after the name in `title_band`, and draw the row as its own top `Panel` under `RIBBON_BAND`.
  - A click pushes `Request::Verb(Verb::new(longname))`, which reaches the arms slice 1 Task 8 adds to `apply`. Task 29 makes it `VerbByPointer`.
  - Show the reason popover on `pointed_unavailable`, or for 4 s after `refuse_key` refuses a key.
  - Add `RIBBON_BAND` to `chrome_budget` and `boot_tile_body`.
  - Record each tab and command as a `NamedControl`.
- [ ] **Step 4: Run the tests, and expect them to pass.** Also run `cargo test -p brightfield-shell --test navigator_spine --test canvas_pane_group --test menu_commands`, where window sizes move and slice 1's sweeps read the handlers.
- [ ] **Step 5: Commit.** Message: "The ribbon: stage tabs, the contextual tab, and commands that say why they are greyed, by pointer and by key".

### Task 7: Collapse, and the dropdown row

**Blocked by:** `AnchoredLayer` (Task 2). It also needs Task 6.

**Files:**
- Modify: `crates/brightfield-shell/src/ribbon.rs`, `crates/brightfield-shell/src/window.rs`, `crates/brightfield-workbench/src/persist.rs`
- Test: `crates/brightfield-shell/tests/ribbon.rs`, `crates/brightfield-workbench/tests/layout_file.rs`

**Interfaces:**
- Produces: `SavedLayout.ribbon_collapsed: bool` with `#[serde(default)]`.

- [ ] **Step 1: Write the failing tests.**
  - `double_clicking_the_selected_tab_puts_the_command_row_away`: `region_rect(RIBBON_BAND)` is `None`, and the canvas grows by `RIBBON_BAND_HEIGHT`.
  - `toggle_ribbon_puts_it_away_and_brings_it_back`: ⌥R.
  - `a_tab_of_a_collapsed_ribbon_opens_its_commands_as_a_dropdown_row`
  - `escape_and_a_click_outside_close_the_dropdown`
  - `choosing_a_command_from_the_dropdown_closes_it_and_runs_the_command`
  - In `layout_file.rs`: `the_collapsed_ribbon_is_remembered_and_an_older_file_opens_it_open`.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test ribbon` and `cargo test -p brightfield-workbench --test layout_file`.
- [ ] **Step 3: Implement.** The dropdown is `AnchoredLayer::show(ctx, "ribbon-dropdown", tab_rect, |ui| ribbon_row(ui, row))`, or `egui::Popup` if the review rules that way (§5). The layout writer picks the field up through the `DirtyTracker` that already debounces it.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The ribbon collapses to its tabs and remembers it".

### Task 8: Key tips while ⌥ is held

**Blocked by:** `key_tip`, drawn by `ribbon_tabs` and `ribbon_row` (Task 2). It also needs Task 7.

**Files:**
- Modify: `crates/brightfield-shell/src/ribbon.rs`
- Test: `crates/brightfield-shell/tests/ribbon.rs`

**Interfaces:**
- Produces: `ribbon::key_tips_shown(ctx: &egui::Context) -> bool`. It is true while `alt` is the only modifier held and `!ctx.wants_keyboard_input()`.

- [ ] **Step 1: Write the failing tests.**
  - `holding_option_puts_a_key_tip_on_every_tab_and_command`: the accesskit labels include each tab's `keys_label` and each drawn command's keycap.
  - `option_typed_into_a_text_field_shows_no_key_tips_and_opens_no_tab`: with the title band's Name field open, ⌥S shows no tip, the selected tab is unchanged, and the field's text changes.
  - `key_tips_still_show_on_a_collapsed_ribbon`
  - `option_and_a_tabs_letter_opens_that_tab`
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test ribbon`.
- [ ] **Step 3: Implement.** Pass `key_tips(key_tips_shown(ctx))` to both components. The tab verbs' handlers set `RibbonState.selected`. On a collapsed ribbon, they open the dropdown.
- [ ] **Step 4: Run the tests, and expect them to pass.** Then check by hand on the Mac Studio with a non-US layout, since no test sees one.
- [ ] **Step 5: Commit.** Message: "Holding ⌥ puts each tab's and command's key on it".

### Task 9: The palette draws a greyed row greyed

This takes on what slice 1 Task 5 leaves to slice 2. The in-window menus from the mark on Linux and Windows, which this task held in an earlier draft, are slice 1 Task 10's.

**Blocked by:** `PickerRow`'s disabled state (§13), adopted in Task 2.

**Files:**
- Modify: `crates/brightfield-shell/src/overlays.rs` (`CommandPalette::row`)
- Test: unit tests in `overlays.rs`; slice 1's `crates/brightfield-shell/tests/overlay_wiring.rs`

**Interfaces:**
- Consumes: slice 1's `PaletteCandidate::missing` and `CommandPalette::with_situation`; `meridian_egui::PickerRow::unavailable`.

- [ ] **Step 1: Write the failing tests.**
  - `a_missing_candidate_is_a_disabled_row_with_its_sentence`: the row's `unavailable == Some("Zoom in needs a chart on the canvas.")`.
  - In `overlay_wiring.rs`, `a_greyed_row_is_drawn_disabled_and_says_its_sentence`: the open palette's accesskit node for the row is disabled, with the sentence as its description.
  - Slice 1's `confirming_a_greyed_row_says_its_sentence_and_runs_nothing` stays green.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell overlays::` and `cargo test -p brightfield-shell --test overlay_wiring`.
- [ ] **Step 3: Implement.** `row()` sets `.unavailable(sentence)` for a candidate whose `missing` is some. The "{help} — {sentence}" detail gives way to the help alone, because the sentence now rides on the row.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "A greyed palette row is drawn greyed".

### Task 10: The palette, grouped by stage

**Blocked by:** nothing from the design system. It needs Task 1 and the operation-forms plan's phase 1 merged, because a form's stage is its `FormSpec`'s.

**Files:**
- Modify: `crates/brightfield-shell/src/overlays.rs` (`CommandPalette`)
- Test: unit tests in `overlays.rs`; `crates/brightfield-shell/tests/typed_sql_palette.rs` stays green

**Interfaces:**
- Consumes: slice 1's `palette_filter(.., situation)` and `CommandPalette::with_situation`; `keys_label`; Task 1's `stage_of`; the forms plan's `forms::by_verb`.
- Produces: `overlays::stage_of_verb(longname: &str) -> Option<Stage>`, which is `forms::by_verb(longname).map(|f| f.stage)`, else `ribbon::stage_of(longname)`.

- [ ] **Step 1: Write the failing tests.**
  - `the_empty_palette_reads_in_stage_order_under_each_stages_name`: `header_before` gives "Connect", "Shape", "Model", "Publish" and then "Other", at the first row of each.
  - `a_typed_query_ranks_by_match_and_draws_no_headers`
  - `the_palette_shows_keys_as_keycaps`: `PickerRow.keystroke == Some(keys_label(primary_key, KeyPlatform::host()))`.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell overlays::`.
- [ ] **Step 3: Implement.** For an empty query, sort stably by `(stage_of_verb(..) position, frequency, recency)`, so a built form sits under its stage with no list naming it. Slice 1's greyed rows keep their place in the sort. The SQL statement path is unchanged.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The palette reads in the ribbon's stages".

---

## Slice 3: the spine's three widths, with the open places and quick open

The slice is finished when the left rail shows the spine thin, as a list or as a graph, by one key. The list's rows carry state, output, timeline and duration. Open places stand above the spine, and ⌘P finds any table or view.

### Task 11: The spine's facts: state, output, timings, lanes and folds

**Blocked by:** nothing from the design system.

**Files:**
- Create: `crates/brightfield-shell/src/spine.rs`
- Modify: `crates/brightfield-protocol/src/stream.rs`, `crates/brightfield-shell/src/protocol.rs` (`SpineRow`, `spine_units`, `spine_with_joins`)
- Test: unit tests in `stream.rs` and `spine.rs`; `crates/brightfield-shell/tests/navigator_spine.rs`

**Interfaces:**
- Produces:
  - In `stream.rs`: `pub struct StepTimes { pub started: Option<String>, pub finished: Option<String> }` and `pub fn step_times(content: &str) -> BTreeMap<String, StepTimes>`. `started` is the first `running` timestamp, and `finished` is the last terminal one.
  - In `spine.rs`:
    - `pub enum SpineState { NotRun, Running, Ok, Failed, Stale }`
    - `pub fn spine_state(seam: SeamStatus, recorded: RunState) -> SpineState`
    - `pub struct Lanes { pub node: Option<u8>, pub through: Vec<u8>, pub from_above: Vec<u8>, pub to_below: Vec<u8> }`
    - `pub fn lanes(rows: usize, nodes: &[Option<usize>], edges: &[(usize, usize)]) -> Vec<Lanes>`
    - `pub struct Timing { pub start: f32, pub end: f32, pub duration: String }`
    - `pub fn timings(times: &BTreeMap<String, StepTimes>) -> BTreeMap<String, Timing>`
  - `SpineRow` gains `state: Option<SpineState>`, `output: Option<String>`, `timing: Option<Timing>`, `fold: Option<(bool, usize)>` and `lanes: Lanes`. `kind` stops carrying the run-state word.
  - `ProtocolDoc.spine_expanded: BTreeSet<AssetId>`: a family listed in it is drawn from `graph_full`.

- [ ] **Step 1: Write the failing tests.**
  - `step_times_keep_the_first_running_and_the_last_terminal_timestamp`
  - `a_chain_sits_on_one_lane`: `lanes` on a→b→c gives every node lane 0 and nothing passing through.
  - `a_fork_opens_a_second_lane_and_the_join_closes_it`: a→{b,c}→d. Rows b and c have nodes on lanes 0 and 1. Row d has `from_above == [0, 1]`, and its node sits on lane 0.
  - `durations_read_in_seconds_to_one_place`: "2.3s", "1m 04s".
  - `a_skip_that_proves_fresh_reads_ok_and_one_that_does_not_reads_not_run`
  - `the_step_rows_run_state_is_the_models_and_not_a_literal` is updated to read `state`.
  - `a_family_folds_to_one_row_and_unfolds_to_its_members`: use a Protocol fixture with a parameterised family, built on `collapse.rs`'s `family_graph` shape if no shell fixture has one.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-protocol stream::`, `cargo test -p brightfield-shell spine::` and `cargo test -p brightfield-shell --test navigator_spine`.
- [ ] **Step 3: Implement `lanes`.** No test fixes the order of lane assignment, so this is it:

  ```text
  pending: Vec<Option<target_row>>   // one per lane
  for r in rows:
      from_above = lanes whose pending target is r; clear them
      node lane  = min(from_above) or the lowest free lane (None if row r has no node)
      through    = lanes still pending
      for each consumer c of r, in row order: take the lowest free lane, set pending = c, push to to_below
  ```

  `start` and `end` are fractions of the span from the earliest `started` to the latest `finished`.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The spine's rows carry their state, output, timing and graph lanes as data".

### Task 12: Adopt slice 3's components

**Blocked by:** `step_row`, `step_mark`, `vertical_tab`, `meridian_design::marks` and `control::STEP_MARK_CELL`.

Do the same as Task 2, with the gallery entries `step-row`, `step-mark` and `vertical-tab`, named for the components the review landed. The pin Task 2 took already carries them, so there is no bump. The gallery's probe roles gain `Row`, and `Unknown` for the step mark. Add Draft demos, draw the goldens on the Mac Studio, promote the entries, and commit with the message "Take the spine's components from the design system".

### Task 13: The list width, drawn with the spine row

**Blocked by:** `step_row` (Task 12). It also needs Task 11.

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs` (`spine_list_ui`; the private `spine_row` is removed; `SpineRowDrawn`)
- Test: `crates/brightfield-shell/tests/navigator_spine.rs`, `spine_beside_the_model.rs`, `outline_list.rs`

**Interfaces:**
- Consumes: `meridian_egui::{step_row, StepRow, Gutter, StepMark, Timeline, Fold, StepEntry}`.
- Produces: `SpineRowDrawn` keeps `rect`, `name_rect`, `washed` and `on_canvas`, and gains `fold_rect: Option<egui::Rect>`. `spine_state_mark(SpineState) -> StepMark` maps the states one to one.

- [ ] **Step 1: Write the failing tests.**
  - `a_step_row_shows_its_duration_and_a_table_row_its_row_count`
  - `the_focused_nodes_row_is_marked_apart_from_the_cursor`
  - `clicking_a_rows_fold_control_unfolds_its_family`
  - `the_spines_measurements_hold_at_both_windows` is re-pinned to `step_row`'s rung.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test navigator_spine --test spine_beside_the_model --test outline_list`.
- [ ] **Step 3: Implement.** Replace the row drawing inside `spine_list_ui` with `step_row`. Caption rows and head rows stay. View rows ("grid") stay as rows with `Gutter { node: None, .. }`, carrying their lanes through.
- [ ] **Step 4: Run the tests, and expect them to pass.** Then redraw `workflow_graph_baseline` and `dashboard_baseline` on the Mac Studio, if their regions show the spine.
- [ ] **Step 5: Commit.** Message: "The spine lists its steps as Flight Explainer does".

### Task 14: The three widths, by one key

**Blocked by:** `step_mark`, `marks::glyph` and `STEP_MARK_CELL` (Task 12). It also needs Task 13.

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/arrangement.rs`
  - `crates/brightfield-shell/src/spine.rs`
  - `crates/brightfield-shell/src/window.rs`: the navigator branches at the door, the model and the dock; `panel_id`.
  - `crates/brightfield-render/src/asset_scene.rs`: the node marks come from `marks::glyph`.
  - `crates/brightfield-keys/src/registry.rs`, `crates/brightfield-keys/src/menus.rs`
- Create: `crates/brightfield-shell/tests/spine_widths.rs`
- Test: also `crates/brightfield-shell/tests/arrangement.rs`

**Interfaces:**
- Produces:
  - In `arrangement.rs`:
    - `pub const NAVIGATOR_THIN_WIDTH: f32 = STEP_MARK_CELL + 2.0 * SPACE_2`.
    - `pub const NAVIGATOR_GRAPH_WIDTH: f32`, a ladder sum the builder picks, about twice `NAVIGATOR_RAIL_WIDTH`.
    - The navigator collapses to `Collapse::To(NAVIGATOR_THIN_WIDTH)`.
  - `pub enum SpineWidth { Thin, List, Graph }` with `next()`.
  - The verb `cycle-spine-width` (Global `cmd-shift-b`), with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm.
  - `MeridianApp::spine_width() -> SpineWidth`.
  - `panel_id(region, state)`, where the state names the width, so each width keeps its own dragged extent.

- [ ] **Step 1: Write the failing tests.**
  - `the_spine_cycles_thin_list_graph_by_one_key`
  - `thin_shows_one_mark_per_step_with_the_focused_one_marked`
  - `the_graph_width_draws_the_dag_in_the_rail`: the rail's canvas slot presents a texture, and a node click focuses it.
  - `each_width_reopens_at_the_extent_it_was_dragged_to`
  - In `arrangement.rs`, `each_rail_collapses_to_the_measure_it_declares` is re-pinned to `NAVIGATOR_THIN_WIDTH`.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test spine_widths --test arrangement`.
- [ ] **Step 3: Implement.**
  - **Thin** replaces `rail_stub` in the navigator: a column of `step_mark`s in the collapsed extent.
  - **Graph** presents the asset graph in a second slot of `ProtocolDoc`'s canvas host, keyed apart from the canvas's, and laid out at rail width with `brightfield_protocol::layout`.
  - `toggle-outline-rail` (⌘B) keeps its focus round trip.
- [ ] **Step 4: Run the tests, and expect them to pass.** Then redraw the asset-graph baselines on the Mac Studio, since the marks moved to `marks::glyph`.
- [ ] **Step 5: Commit.** Message: "The spine is thin, a list, or the graph, by one key".

### Task 15: The open places, above the spine

**Blocked by:** `vertical_tab` (Task 12). It also needs Task 13.

**Files:**
- Modify: `crates/brightfield-shell/src/spine.rs`, `crates/brightfield-shell/src/window.rs`, `crates/brightfield-keys/src/registry.rs`, `crates/brightfield-keys/src/menus.rs`, `crates/brightfield-keys/src/keystroke.rs` (`[` and `]` join `KEY_NAMES`)
- Create: `crates/brightfield-shell/tests/open_places.rs`

**Interfaces:**
- Produces:
  - `pub struct Place { pub node: AssetId, pub view: PlaceView }`, with `pub enum PlaceView { Grid, Chart, Model, Graph, Dashboard }`, read from `CanvasHolds` and the model pane.
  - `pub struct OpenPlaces { places: Vec<Place>, current: usize }`, with `open(&mut self, Place)`, `close(&mut self, usize) -> Option<Place>`, `next()` and `prev()`.
  - The verbs `next-place` (`cmd-shift-]`), `prev-place` (`cmd-shift-[`) and `close-place` (`cmd-w`), all Global, each with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm.

- [ ] **Step 1: Write the failing tests.**
  - `focusing_a_node_adds_its_place_once`
  - `clicking_a_place_brings_its_node_and_view_back`
  - `closing_the_current_place_moves_to_the_one_above`
  - `the_last_place_cannot_be_closed_out_of_the_window`: ⌘W on the only place leaves it, and the window stays.
  - `next_and_previous_place_walk_the_list`
  - Also in `OpenPlaces`, the unit test `closing_a_place_above_the_current_one_keeps_the_current_one`.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test open_places`.
- [ ] **Step 3: Implement.** The navigator's list and graph widths split their body. The places come first, at their measured height, and then the spine. The thin width shows no places.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "Open places stand above the spine, as vertical tabs".

### Task 16: Quick open, by DuckDB name

**Blocked by:** nothing from the design system. `Picker` exists.

**Files:**
- Modify: `crates/brightfield-shell/src/overlays.rs`, `crates/brightfield-shell/src/window.rs` (`open_jump`; `Overlay`), `crates/brightfield-keys/src/registry.rs`, `crates/brightfield-keys/src/menus.rs`
- Create: `crates/brightfield-shell/tests/quick_open.rs`

**Interfaces:**
- Produces:
  - `pub struct QuickOpen(JumpList)`, a `PickerDelegate` over every table and view in the Protocol. Each row's label is the DuckDB relation name, and its detail is the producing step.
  - The verb `quick-open` (Global `cmd-p`), with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm.
  - `Overlay::QuickOpen`.

- [ ] **Step 1: Write the failing tests.**
  - `cmd_p_finds_a_table_by_its_duckdb_name_from_any_focus`
  - `choosing_a_table_focuses_it_in_the_current_activity`: a grid stays a grid.
  - `quick_open_is_offered_off_the_graph_too`, because today's `/` works only while the graph holds the canvas.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test quick_open`.
- [ ] **Step 3: Implement.** Generalise `open_jump`'s corpus from `model.outline()` to every tabular asset. Confirming sets `canvas_pick` with the current `PlaceView`. `/` (`focus-jump`) keeps its meaning.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "⌘P opens any table or view by name".

---

## Slice 4: the activity switcher bound to the node, and the split

The slice is finished when:
- each pane header names `step › node`, offers its node's activities with their keys, and ends with the node's kind and size;
- the canvas splits on ⌘\;
- the locator band and the ledger's Rows spot are gone.

The header is then two rows open and one collapsed.

### Task 17: Activities bound to the node

**Blocked by:** nothing from the design system. It needs Task 1, because `relate` joins `Stage::Model`.

**Files:**
- Create: `crates/brightfield-shell/src/node_activity.rs`, `crates/brightfield-shell/tests/activity_switcher.rs`
- Modify: `crates/brightfield-shell/src/protocol.rs` (`NodeView`, `charts_reading`), `crates/brightfield-shell/src/overlays.rs` (`ChooseChart`), `crates/brightfield-shell/src/window.rs`, `crates/brightfield-keys/src/registry.rs`, `crates/brightfield-keys/src/ribbon.rs`, `crates/brightfield-keys/src/menus.rs`

**Interfaces:**
- Produces:
  - `NodeView::{Grid, Chart}`.
  - `ProtocolModel::charts_reading(&self, table: &AssetId) -> Vec<AssetId>`, which replaces `dashboard()`'s first-wins read for this purpose.
  - `pub enum NodeActivity { Grid, Chart, Model }`.
  - `pub struct ActivityOffer { pub activity: NodeActivity, pub verb: &'static str, pub unavailable: Option<String> }`. `unavailable` is the `Missing` sentence of slice 1's `availability_of(verb)`, so the switcher, the ribbon and the menus say one sentence.
  - `pub fn offers(node: &NodeFacts, app: &MeridianApp) -> Vec<ActivityOffer>`, where:
    - a table or view gets Grid and Chart;
    - a chart gets Grid, which shows the rows it reads, and Chart;
    - the model step gets Model.
  - The verbs `show-grid` (`cmd-alt-g`), `show-chart` (`cmd-alt-c`) and `relate` (`cmd-alt-m`), each with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm. `relate` focuses the model view, has the label "Relate", declares `Sources(2)` with "Relate needs two sources.", and joins `Stage::Model` in the ribbon's taxonomy.
  - `pub struct ChooseChart`, a `PickerDelegate`.

- [ ] **Step 1: Write the failing tests.**
  - `switching_from_chart_to_grid_and_back_keeps_the_node_and_the_selection`
  - `chart_opens_the_one_chart_that_reads_the_node`
  - `chart_asks_which_when_several_charts_read_the_node`: `ChooseChart` lists them.
  - `chart_starts_an_unsaved_draft_when_none_reads_the_node`: the pane's `Dirty` is `Edited`, and the draft is kept on ⌘S.
  - `leaving_an_unsaved_draft_asks_first`, through the existing close question.
  - `relate_needs_two_sources`: with one source, `availability_of("relate")` and the offer both say "Relate needs two sources."
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test activity_switcher`.
- [ ] **Step 3: Implement.**
  - The draft is the generated dashboard `hold_table` builds for a data file, held unsaved.
  - `CanvasHolds::View { view: NodeView::Chart, .. }` draws it, through the existing `draw_chart_pane`.
  - The keys act now, with nothing to click yet. Task 18 adds the switcher.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "Grid and Chart are activities of the node in focus".

### Task 18: The pane header carries the crumb, the switcher and the size, and the locator band goes

**Blocked by:** `activity_switcher` landed in `meridian-online/design`. This task bumps the pin and adds the `activity-switcher` gallery entry, as Task 2 does. It also needs Task 17.

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/chrome.rs` (`header_band`, `pane_frame`)
  - `crates/brightfield-workbench/src/subject.rs`
  - `crates/brightfield-workbench/src/arrangement.rs`: remove `LOCATOR_BAND` and `LOCATOR_BAND_HEIGHT`.
  - `crates/brightfield-shell/src/window.rs`: remove `locator_band_ui`'s band; `chrome_budget`; `boot_tile_body`; the crumb link moves.
  - `gallery.rs`, `Cargo.toml`
- Test:
  - `crates/brightfield-shell/tests/activity_switcher.rs`
  - Re-pin in `navigator_spine.rs`:
    - `the_locator_band_reads_the_file_the_step_the_node_and_the_view` becomes `the_pane_header_reads_the_step_and_the_node`.
    - `the_locator_band_names_the_dashboard_as_the_node_it_is` and `the_locator_band_reads_protocol_and_the_graphs_own_counts` likewise.
  - In `protocol_panels_window.rs`, `the_protocols_crumb_goes_back_to_the_tiles` now clicks the header's first crumb.
  - In `canvas_pane_group.rs`, `the_canvas_draws_no_band_of_its_own_above_the_panes` now reads the header.
  - Also `arrangement.rs`, `one_window.rs`, `chart_contract.rs`, `workflow_graph_baseline.rs`.

**Interfaces:**
- Consumes: `meridian_egui::{activity_switcher, ActivitySwitcher, ActivityOption}`; `offers`.
- Produces:
  - `Subject.activities: Option<(Vec<ActivityChoice>, usize)>`, where `pub struct ActivityChoice { pub label: &'static str, pub verb: Verb, pub unavailable: Option<String> }`. This is plain data, with no egui type.
  - `Subject.size_note: Option<String>`, such as "table · 19,028 rows".
  - `Subject.breadcrumb` now holds `[step, node]`.
  - `MeridianApp::header_crumbs(PaneKey) -> Vec<String>`, which replaces `locator_crumbs()`.

- [ ] **Step 1: Write the failing tests.**
  - `the_pane_header_names_the_node_and_the_step_that_made_it`: "rank › ranking".
  - `the_header_offers_grid_and_chart_with_their_keys`
  - `an_activity_the_node_cannot_take_is_greyed_with_its_reason`
  - `the_headers_right_end_says_the_nodes_kind_and_size`
  - `the_open_header_is_two_rows`: no band between `RIBBON_BAND` and the canvas, the rails and the panes.
  - `the_first_crumb_goes_back_to_every_panel`
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test activity_switcher --test navigator_spine --test protocol_panels_window --test canvas_pane_group --test arrangement`.
- [ ] **Step 3: Implement.**
  - `header_band` draws, left to right: `breadcrumb`, then `activity_switcher`, then `size_note` right-aligned beside the dirty dot.
  - Each of the switcher's clicks becomes a `Request::Verb(choice.verb)`.
  - Drop the locator band's region, draw call and sizing terms.
- [ ] **Step 4: Run the tests, and expect them to pass.** Then redraw the affected baselines on the Mac Studio.
- [ ] **Step 5: Commit.** Message: "The pane header names its node and switches its activity, and the locator band goes".

### Task 19: The split

**Blocked by:** `activity_switcher` (Task 18), because each pane carries its own.

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (`CanvasPanes`, `CanvasPane`, `canvas_pane_rects`, `draw_canvas_pane_group`), `crates/brightfield-keys/src/registry.rs`, `crates/brightfield-keys/src/menus.rs`, `crates/brightfield-keys/src/keystroke.rs`
- Test: `crates/brightfield-shell/tests/canvas_pane_group.rs`, `crates/brightfield-shell/tests/activity_switcher.rs`

**Interfaces:**
- Produces:
  - `CanvasPane.place: Place`, which replaces `name: &'static str`. `CanvasPanes::pane_at(Place)` replaces `pane(name)`.
  - The verb `split-canvas` (Global `cmd-\`, consumed as `Key::Backslash`), with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm. `\` joins slice 1's `KEY_NAMES` in `crates/brightfield-keys/src/keystroke.rs`.
  - `MeridianApp::focused_pane() -> Option<Place>`.

- [ ] **Step 1: Write the failing tests.**
  - `split_opens_the_same_node_in_the_other_activity_beside_it`
  - `the_activity_keys_act_on_the_pane_with_focus`
  - `each_pane_of_a_split_keeps_its_own_header`
  - `split_again_closes_the_pane_without_focus`
  - The existing `the_hero_and_the_grid_split_the_canvas_evenly_side_by_side`, `the_dragged_pane_edge_comes_back_on_the_next_open` and `neither_pane_can_be_dragged_shut` hold for a dashboard's chart beside its table's grid.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test canvas_pane_group --test activity_switcher`.
- [ ] **Step 3: Implement.** Generalise the generated dashboard's hero-and-grid pair to two `Place`s. Keep `SavedLayout.canvas_split` as the dividing fraction.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The canvas splits in two on request, each pane with its own activity".

### Task 20: Retire the ledger's Rows spot and `move-grid`

**Blocked by:** nothing from the design system. It needs Task 19, so the grid beside a chart is already a split.

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/arrangement.rs` (`LEDGER_PANES`, `LEDGER_GRID_ROWS`)
  - `crates/brightfield-workbench/src/persist.rs` (`GridSpot`)
  - `crates/brightfield-shell/src/data_grid.rs` (`RowsSpot`)
  - `crates/brightfield-shell/src/window.rs` (`move_grid`, `set_grid_spot`, `draw_ledger_grid_pane`, the `cmd-j` arm, `situation()`'s `Selected::Rows` reading stays)
  - `crates/brightfield-keys/src/registry.rs`
  - `crates/brightfield-keys/src/menus.rs`: `move-grid` leaves slice 1's Window menu.
- Test: `crates/brightfield-shell/tests/versions_panel.rs`, `grid_spot.rs`, `crates/brightfield-workbench/tests/layout_file.rs`; slice 1's `crates/brightfield-keys/src/menus.rs` tests and `crates/brightfield-shell/tests/menu_commands.rs`

**Interfaces:**
- Produces:
  - `LEDGER_PANES` is `[protocol-log, protocol-quality, spec-editor, chart-versions]`.
  - `GridSpot::Ledger` stays deserialisable and is read as `Canvas`.
  - `move-grid` leaves the registry.
  - The `chart-rows` id stays published, so an old layout parses.

- [ ] **Step 1: Write the failing tests.**
  - `a_layout_that_kept_the_grid_in_the_ledger_opens_with_it_on_the_canvas`
  - `the_ledger_strip_names_four_panels`, renamed from `the_strip_names_five_panels_and_a_click_on_versions_opens_the_rail_at_the_ledgers_extents`.
  - `cmd_j_is_free`: no verb binds it.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test versions_panel --test grid_spot --test menu_commands --test keys_answered`, `cargo test -p brightfield-workbench --test layout_file` and `cargo test -p brightfield-keys`.
- [ ] **Step 3: Implement.** Delete the Rows spot's code paths. `the_grid_context_moves_the_cursor_a_cell_on_each_of_its_keys` stays green.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The grid lives on the canvas; the ledger's Rows spot is retired".

---

## Slice 5: the inspector

The slice is finished when the right rail is one inspector. It describes a column (what it is, its values and what can be done with it), a chart (its encoding, what it reads, its fields and parameters) and the step, in titled sections with counts.

### Task 21: Column profiles for any focused table, with the upper quartile

**Blocked by:** nothing from the design system.

**Files:**
- Modify: `crates/brightfield-engine/src/profile.rs` (`ColumnMoments`), `crates/brightfield-engine/src/lib.rs` (`profile_sources`, `profile_columns_with`), `crates/brightfield-shell/src/inspector.rs` (`ColumnTable`), `crates/brightfield-shell/src/window.rs`
- Test: `crates/brightfield-engine` unit tests; `crates/brightfield-shell/tests/inspector_contract.rs`

**Interfaces:**
- Produces:
  - `ColumnMoments.upper_quartile: Option<f64>`, from `quantile_cont(col, 0.75)` in the aggregate statement.
  - `MeridianApp::column_facts_for(table: &str) -> Option<&[ColumnFacts]>`. It profiles the table on a worker the first time it is asked, and caches the result by `(table, AssetMeta.content_hash)`.
  - `ColumnTable { step, kind, state }`, read from the model's producing step rather than "load", `NOT_RUN`.

- [ ] **Step 1: Write the failing tests.**
  - In the engine, `the_profile_carries_the_upper_quartile`: a column holding 1..=100 gives `upper_quartile == Some(75.25)`.
  - In the shell:
    - `a_column_of_a_steps_table_is_profiled_when_first_inspected`
    - `the_inspector_names_the_step_that_made_the_column_and_its_state`
    - `a_profile_still_reading_says_so_rather_than_showing_nothing`, which is the 100 ms honesty line.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-engine profile` and `cargo test -p brightfield-shell --test inspector_contract`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "Profile any table the inspector is asked about, upper quartile included".

### Task 22: Adopt slice 5's components

**Blocked by:** `inspector_section`, `distribution_bar` and `menu_row`.

Do the same as Task 2, with the gallery entries `inspector-section`, `distribution-bar` and `menu-row`. Commit with the message "Take the inspector's components from the design system".

### Task 23: One inspector, in sections

**Blocked by:** `inspector_section` (Task 22).

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/arrangement.rs`: `INSPECTOR_PANES` becomes `[ItemId::new("inspector")]`.
  - `crates/brightfield-shell/src/inspector.rs`
  - `crates/brightfield-shell/src/window.rs`: `inspector_panel` goes. `open_rail_on_operation` and `restore_rail_after_ask` keep their names, because the operation forms call them, and now open and close the Operation section. `apply_rail_defaults` changes to match.
  - `crates/brightfield-shell/src/protocol.rs` (`InspectorPane`'s body becomes the Step section)
  - `crates/brightfield-shell/src/operation_panel.rs`: its body becomes the Operation section. This is the panel the operation-forms plan's Task 6 draws.
  - `crates/brightfield-shell/src/app.rs` (`publish_item_ids`)
  - `crates/brightfield-keys/src/registry.rs`: `toggle-inspector-rail` becomes Built on Global `cmd-alt-b`, with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm. `toggle-controls-rail` is removed, and `reserved_buckets_present_with_reasons` is updated.
  - `crates/brightfield-keys/src/menus.rs`
- Test: `crates/brightfield-shell/tests/inspector_contract.rs`, `operation_panel.rs`, `chart_contract.rs`, `arrangement.rs`

**Interfaces:**
- Produces:
  - `pub enum Section { Operation, Column, Encoding, Reads, Fields, Parameters, Step }`.
  - `pub fn sections_for(selection: &InspectorSelection) -> Vec<Section>`, in the order "Decisions" gives.
  - `InspectorPane::open: BTreeMap<Section, bool>`.
  - The ids `protocol-inspector`, `chart-controls` and `operation-inspector` stay published.

- [ ] **Step 1: Write the failing tests.**
  - `the_inspector_rail_draws_one_pane_and_no_tab_strip`
  - `w_opens_the_operation_section_at_the_top`, renamed from `w_opens_the_inspector_on_the_operation_panel`.
  - `escape_closes_the_operation_section`, renamed from `escape_shuts_the_rail_it_opened`.
  - `the_step_section_is_last_and_collapsed_while_another_is_open`
  - `a_layout_saved_with_three_inspector_panes_opens_on_the_one_inspector`
  - `cmd_alt_b_hides_and_brings_back_the_inspector`
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test inspector_contract --test operation_panel --test chart_contract --test arrangement`.
- [ ] **Step 3: Implement.** Each section's body is today's pane body, moved. `the_controls_rail_is_drawn_by_the_inspector_pane` keeps its meaning under the new id.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The right rail is one inspector that describes the selection".

### Task 24: The column's sections

**Blocked by:** `distribution_bar` and `menu_row` (Task 22). It also needs Tasks 21 and 23, and the operation-forms plan's phase 1 merged.

**Files:**
- Modify: `crates/brightfield-shell/src/inspector.rs` (`column_body`)
- Test: `crates/brightfield-shell/tests/inspector_contract.rs`

**Interfaces:**
- Consumes: `meridian_egui::{distribution_bar, DistributionBar, Bars, menu_row, MenuRow, widgets::field_row}`; `ColumnMoments::bars()`; slice 1's `availability_of`, `VerbEntry::label` and `keys_label`; the forms plan's `forms::built()`.

- [ ] **Step 1: Write the failing tests.**
  - `the_column_section_says_its_finetype_its_storage_type_and_its_step`
  - `a_numeric_column_shows_its_bins_and_minimum_median_upper_quartile_maximum_and_missing`
  - `a_categorical_column_shows_its_top_values`
  - `what_can_be_done_lists_the_columns_commands_with_their_keys`. The list is:
    - the built forms asked from the grid, read from `forms::built()` so a form joins with no edit here, filter and sort among them;
    - `put-column-on-x`, `put-column-on-y`, `put-column-on-colour` and `retype-column`.

    Each is a `meridian_egui::menu_row` with its label and `keys_label`, greyed with its `availability_of` sentence.
  - `hovering_a_bin_says_its_range_and_count`
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test inspector_contract`.
- [ ] **Step 3: Implement.** Show `Bars::Bins` for a number, and `Bars::Top` for a category of 64 or fewer distinct values (`Distribution::Values`). Draw each figure with `field_row` inside an `align_scope`.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The inspector describes a column as Column Explorer does".

### Task 25: The chart's sections

**Blocked by:** `inspector_section` and `menu_row` (Task 22). It also needs Task 23.

**Files:**
- Modify: `crates/brightfield-shell/src/inspector.rs`
- Test: `crates/brightfield-shell/tests/inspector_contract.rs`

- [ ] **Step 1: Write the failing tests.**
  - `the_encoding_section_lists_each_channel_and_its_field`: each is a `menu_row` whose key is the shelf's `go-to-*-cell` key, and which focuses that shelf cell when clicked.
  - `the_reads_section_names_the_table_and_the_rows_drawn`: "ranking · 19,028 of 19,028 rows".
  - `the_fields_section_counts_the_fields_available`
  - `the_parameters_section_holds_the_sliders`, moved from the old Inspector pane.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test inspector_contract`.
- [ ] **Step 3: Implement.** The shelf stays in the chart pane as the place the encoding is edited. The Encoding section reads it and points at it.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The inspector describes a chart: its encoding, what it reads, its fields".

---

## Slice 6: the console and the status band that teaches keys

The slice is finished when:
- the bottom rail is the console, with Run log, Problems, Versions and Editor;
- collapsed, the console is one line holding the latest entry, and ⌘J opens and closes it;
- during a run, each step's line arrives as the step finishes;
- the status band always names the node, the activity and the selection, with the counts and the engine's state;
- after a mouse use, the status band names the command's key.

### Task 26: The console's panes and its key

**Blocked by:** nothing from the design system. It needs Task 20.

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/arrangement.rs`: `LEDGER_PANES` becomes Run log, Problems, Versions, Editor; the doc names it the console; the ids are unchanged.
  - `crates/brightfield-workbench/src/chrome.rs`: `Trailing` gains `keycap: Option<&str>`, drawn with `key_chip`.
  - `crates/brightfield-shell/src/protocol.rs`: `LogPane` and `QualityPane`'s titles, and `QualityPane`'s Problems content.
  - `crates/brightfield-shell/src/window.rs`: `ledger_summary` becomes `latest_log_entry`.
  - `crates/brightfield-keys/src/registry.rs`
  - `crates/brightfield-keys/src/menus.rs`: `toggle-console` takes `move-grid`'s place in slice 1's Window menu.
- Create: `crates/brightfield-shell/tests/console.rs`

**Interfaces:**
- Produces:
  - The verb `toggle-console` (Global `cmd-j`), with its label, needs and menu from "Decisions", recorded in `keys_answered` by its key handler and in `verbs_applied` by its `apply` arm.
  - `open-run-log`, `open-run-quality` and `open-chart-versions` leave `ReservedReason::NeedsWorkspaceShell`, becoming Built and unbound, with the labels "Run log", "Problems" and "Versions" and the need `Selected::Protocol`. Each opens the console on its tab. `reserved_buckets_present_with_reasons` is updated.
  - `MeridianApp::latest_log_entry() -> Option<String>`.

- [ ] **Step 1: Write the failing tests.**
  - `the_console_names_run_log_problems_versions_and_source`
  - `collapsed_the_console_is_one_line_with_the_latest_entry_and_its_key`
  - `cmd_j_opens_and_closes_the_console`
  - `problems_lists_the_failed_steps_of_the_last_run_with_their_messages`
  - `problems_says_so_when_the_last_run_had_none`
  - `the_palette_opens_the_console_on_a_tab`
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test console --test versions_panel --test one_step_protocol --test keys_answered --test menu_commands` and `cargo test -p brightfield-keys`.
- [ ] **Step 3: Implement.** Problems reads the failed `StepView`s, and their issue text, from `model.run`.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The bottom rail is the console: run log, problems, versions and source".

### Task 27: Each step's line as it finishes

**Blocked by:** nothing from the design system. It needs Tasks 4 and 26.

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (the run poll), `crates/brightfield-shell/src/protocol.rs` (`LogPane`, `set_run_log`)
- Test: `crates/brightfield-shell/tests/console.rs`

**Interfaces:**
- Produces: `ProtocolModel::push_step_line(&mut self, line: String)`. Each line reads `"{step} · {word} · {duration}"`, using `status_word`'s words.

- [ ] **Step 1: Write the failing tests.**
  - `during_a_run_the_console_takes_each_steps_line_as_it_finishes`: driven by a stream file the test appends to between frames.
  - `when_the_run_ends_the_captured_log_follows_the_step_lines`
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test console`.
- [ ] **Step 3: Implement.** On each poll, diff the folded `StreamState` against the steps already reported. For a step that turned terminal, push its line.
- [ ] **Step 4: Run the tests, and expect them to pass.**
- [ ] **Step 5: Commit.** Message: "The console takes each step's line as the step finishes".

### Task 28: The status band becomes a band, and the hint band folds into it

**Blocked by:** nothing from the design system. It needs Task 26.

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/arrangement.rs`:
    - `STATUS_BAND` becomes `Extent::Band(chrome::status_rail_height())` with `RegionFrame::Band`;
    - `LEDGER_RAIL` becomes `Collapse::To(chrome::rail_selector_height())`;
    - `HINT_BAND` and `HINT_BAND_HEIGHT` are removed.
  - `crates/brightfield-workbench/src/chrome.rs`: `status_rail_overlay` becomes `status_band(ui, entries, centre, mode)`, and `HideAffordance::Transient` is enforced.
  - `crates/brightfield-shell/src/window.rs` (`status_rail_ui`, the `hint_ui` call, `chrome_budget`)
  - `crates/brightfield-shell/src/protocol.rs` (`hint_ui` becomes `graph_grammar_line`)
- Test: `crates/brightfield-shell/tests/status_rail.rs`, `crates/brightfield-shell/tests/arrangement.rs`, `crates/brightfield-workbench/tests/chrome_rules.rs`, the unit tests in `arrangement.rs`

**Interfaces:**
- Produces:
  - `pub enum StatusCentre<'a> { None, Grammar(&'a str), Teaching { command: &'a str, keystroke: &'a str } }`.
  - `chrome::status_band(ui: &mut egui::Ui, entries: &[StatusEntry], centre: StatusCentre<'_>, mode: Mode) -> StatusDrawn`.
  - `MeridianApp::status_lead() -> String`, which reads `"{node} · {activity} · {selection}"`, for example "ranking · Grid · column installs_30d".

- [ ] **Step 1: Write the failing tests.**
  - `the_status_band_always_names_the_node_the_activity_and_the_selection`
  - `the_status_bands_right_end_says_the_count_and_whether_the_engine_is_idle`: "19,028 rows · idle", and "working" while a query runs.
  - `the_graphs_key_grammar_sits_in_the_status_bands_centre`
  - `a_transient_entry_goes_when_its_time_is_up`
  - The workbench test `the_overlay_region_takes_no_space_and_the_rest_do` becomes `no_shipped_region_floats`.
  - `the_collapsed_ledger_keeps_its_selector_strip_and_reopens_from_it` is re-pinned to the new collapsed extent.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test status_rail --test arrangement` and `cargo test -p brightfield-workbench`.
- [ ] **Step 3: Implement.** Existing leading entries follow `status_lead`, and existing trailing entries keep their side. The status band is drawn as a bottom `Panel`, added before `LEDGER_RAIL`.
- [ ] **Step 4: Run the tests, and expect them to pass.** Then redraw `status_rail` baselines and the window baselines on the Mac Studio.
- [ ] **Step 5: Commit.** Message: "The status band is a band that always says where the window is".

### Task 29: The status band teaches a command's key after mouse use

**Blocked by:** `teaching_message` landed in `meridian-online/design`. This task bumps the pin and adds the `teaching-message` gallery entry, as Task 2 does. It also needs Task 28.

**Files:**
- Modify:
  - `crates/brightfield-workbench/src/item.rs` (`Request`, `ItemCtx`)
  - `crates/brightfield-workbench/src/chrome.rs`: `toolbar_button` pushes `VerbByPointer`; `status_band`'s Teaching arm.
  - `crates/brightfield-shell/src/window.rs`: `apply`; the teaching state; the ribbon, switcher, places and inspector click sites; and the drain of slice 1's `OpenQueue::take_verbs`.
  - `crates/brightfield-shell/src/menu_bar.rs` (slice 1's): a choice in `show_menus` becomes `Request::VerbByPointer`.
  - `gallery.rs`, `Cargo.toml`
- Create: `crates/brightfield-shell/tests/teaching.rs`

**Interfaces:**
- Consumes: `meridian_egui::{teaching_message, TeachingMessage}`; slice 1's `VerbEntry::label`, `keys_label`, `OpenQueue::take_verbs` and `show_menus`; Task 6's `dispatch_context`.
- Produces:
  - `Request::VerbByPointer(Verb)`.
  - `ItemCtx::request_by_pointer(verb)`.
  - `MeridianApp::teaching() -> Option<(String, String)>`, which holds for `TEACH_FOR = Duration::from_secs_f64(meridian_egui::notify::DEFAULT_TOAST_SECONDS)`.

- [ ] **Step 1: Write the failing tests.**
  - `after_filter_rows_is_clicked_the_status_band_says_next_time_press_w`: the band's text holds "Filter rows · next time, press W".
  - `the_same_command_by_its_key_teaches_nothing`
  - `the_message_goes_after_four_seconds`: driven by a test clock seam, `MeridianApp::set_clock`.
  - `a_command_with_no_key_teaches_nothing`
  - `the_next_mouse_use_replaces_the_message`
  - `a_command_chosen_from_a_menu_teaches_its_key`: a verb asked through `OpenQueue::ask_verb` teaches, as a click does. Slice 1 counts a menu choice as a pointer use.
- [ ] **Step 2: Run them, and expect failure.** Run `cargo test -p brightfield-shell --test teaching`.
- [ ] **Step 3: Implement.** `apply` treats `VerbByPointer(v)` as `Verb(v)`. When the verb has a binding that fires in the current dispatch context, it also sets the teaching state.
- [ ] **Step 4: Run the tests, and expect them to pass.** Also run `cargo test -p brightfield-workbench`, and `cargo test -p brightfield-shell --test menu_commands --test mark_menu`.
- [ ] **Step 5: Commit.** Message: "After a mouse use, the status band names the command's key".

### Task 30: Fold the spec and this plan into the architecture docs

**Blocked by:** Tasks 1 to 29.

**Files:**
- Modify: `README.md` (a "### The chrome" section under "## Architecture"), and the module docs of `crates/brightfield-workbench/src/arrangement.rs`, `crates/brightfield-workbench/src/chrome.rs` and `crates/brightfield-keys/src/ribbon.rs`
- Delete: `docs/superpowers/specs/2026-11-21-brightfield-chrome-design.md` and `docs/superpowers/plans/2026-11-22-chrome-slices-2-to-6.md`

- [ ] **Step 1: Write the "### The chrome" section.** It covers the regions as they now stand, and the owner's rulings from the spec's brainstorm with their reasons, written in plain words. It also covers the owner's answers to the items flagged above, and this plan's decisions.
- [ ] **Step 2: Check the comments.** Run `cargo xtask ci --step comment-citations` after the commit. Expect a pass.
- [ ] **Step 3: Commit.** Message: "Fold the chrome's design into the architecture docs".

---

## Dependency summary

| Task | Waits on in the design system | Waits on in this plan, slice 1 or the forms plan |
|---|---|---|
| 1 Taxonomy and keys | none | slice 1 (`label`, `needs`, `keys_label`, `KeyPlatform`, `menu_bar`, `keys_answered`) |
| 2 Adopt slice 2's components | ribbon tabs, ribbon row, key tip, reason popover, anchored layer (or `egui::Popup`), activity area, `PickerRow`'s disabled state, icons | none |
| 3 Unified title band | none | slice 1 (`MenuHost`, `mark_rect`) |
| 4 Run progress | none | none |
| 5 Activity area and Run | activity area, ribbon row | 1, 2, 4; slice 1 (`Condition`, `situation`) |
| 6 Ribbon tabs, row and reasons, by pointer and by key | ribbon tabs, ribbon row, reason popover | 1, 2, 3, 5; slice 1 (`situation`, `availability_of`); forms phase 1 (`forms::built`, with filter and sort) |
| 7 Collapse and dropdown | anchored layer (or `egui::Popup`) | 6 |
| 8 Key tips | key tip | 7 |
| 9 Greyed palette rows drawn greyed | `PickerRow`'s disabled state | 2; slice 1 (`PaletteCandidate::missing`) |
| 10 Palette by stage | none | 1; slice 1 (`palette_filter` with a situation); forms phase 1 (`forms::by_verb`) |
| 11 Spine facts | none | none |
| 12 Adopt slice 3's components | step row, step mark, marks data, `STEP_MARK_CELL`, vertical tab | none |
| 13 List width | step row | 11, 12 |
| 14 Three widths | step mark, marks data | 13; slice 1 (`menus.rs`) |
| 15 Open places | vertical tab | 13; slice 1 (`menus.rs`, `KEY_NAMES`) |
| 16 Quick open | none | slice 1 (`label`, `needs`, `menus.rs`) |
| 17 Activities | none | 1; slice 1 (`availability_of`) |
| 18 Pane header; locator band goes | activity switcher | 17 |
| 19 Split | activity switcher | 18; slice 1 (`KEY_NAMES`) |
| 20 Retire Rows and `move-grid` | none | 19; slice 1 (`menus.rs`) |
| 21 Column profiles | none | none |
| 22 Adopt slice 5's components | inspector section, distribution bar, menu row | none |
| 23 One inspector | inspector section | 22; the forms plan's Operation panel (its Task 6) |
| 24 Column sections | distribution bar, menu row | 21, 22, 23; forms phase 1 |
| 25 Chart sections | inspector section, menu row | 22, 23 |
| 26 Console panes and key | none | 20 |
| 27 Step lines | none | 4, 26 |
| 28 Status band as a band | none | 26 |
| 29 Teaching | teaching message | 6, 28; slice 1 (`OpenQueue`, `show_menus`) |
| 30 Fold | none | 1–29 |

- **Can start as soon as slice 1 is merged:** Tasks 3, 4, 11, 16 and 21. Task 4, 11 and 21 need nothing of slice 1 and can start sooner.
- **Need slice 1 first:** Task 1. Task 17 also needs Task 1.
- **Wait on the operation-forms plan's phase 1** (its Tasks 1–6: the forms registry, with filter and sort as forms): Tasks 6, 10 and 24.
  - No task here waits on a later phase.
  - No form task edits a file of this plan's: a form reaches the ribbon, the palette's grouping and the inspector's column commands by flipping its `*_BUILT` flag.
- **The design system.** Each slice's blocked tasks wait for the review's release of their components.
