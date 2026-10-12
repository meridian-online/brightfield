# Colour's scale, domain and legend rows: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Colour's settings list gains a scale row (linear, diverging about a pivot, steps in a count), a domain row (auto, fixed, two ends), a legend row (right, below, none) and a ring round the legend while the list is open. A window test holds the reverse row's hand-off.

**Architecture:** The rows are built in `brightfield-shell/src/shelf.rs` (`colour_rows`). A kept row becomes a `RowEdit`, and `app.rs` (`axis_row_written`) turns it into `ChartEdit`s through `shelf_edit.rs`. The render crate gains a colour pin in `PinnedDomains`, read from `colorDomain: Fixed`, and recognises brightfield's own diverging pair in a `colorRange`. The legend draw functions return the rects they drew, so `chart_item.rs` can ring the block.

**Tech Stack:** Rust 1.95.0 (`cargo +1.95.0`), egui/eframe, `egui_kittest` snapshots, DuckDB through `brightfield-engine`.

**Spec:** `docs/superpowers/specs/2026-10-12-colour-shelf-settings-design.md`

## Global Constraints

- This repo is public. No planning identifiers in code, comments, commit messages or PR text; write the rationale in plain words.
- `cargo xtask ci` is the gate before every commit. `cargo xtask ci --step <name>` re-runs one step.
- Never run `cargo fmt --all` on a toolchain other than 1.95.0; the gate's `fmt` step is the check.
- A baseline the change moves is redrawn with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test <file>`, read, and named in the PR with what moved. `kittest.toml`'s thresholds (`threshold = 0.6`, `failed_pixel_count_threshold = 0`) are not loosened.
- Before a push: `cargo +1.95.0 test --locked --workspace --no-fail-fast -- --test-threads=1`, totals quoted in the PR. Two `remote_start` tests fail on a Mac on `main`; name them as such.
- Check free disk (`df -h ~`) before building a worktree beside another; a parallel build was measured wanting about 89 GiB.
- Comments match the surrounding code's density and voice: full sentences that say why.
- The diverging pair as the file writes it: `colorRange: ['<hex of DIVERGING_BLUE_ARM[0]>', '<hex of DIVERGING_MID_LIGHT>', '<hex of DIVERGING_RED_ARM[4]>']`, from `meridian_design::viz`.
- `DEFAULT_COLOUR_STEPS = 5` and `MAX_COLOUR_STEPS = 256` (`brightfield-spec/src/layout.rs`) bound the count.

## Review Focus

1. **A hand-written file with `colorScale: diverging` and no `colorPivot`**: the scale row reads the pivot the chart drew, muted, and stepping off and back on does not mark it set. Pinned by a test in Task 3.
2. **The scale row stepped on a colour that paints names**: the row is muted with its reason and `←` `→` do nothing; no `colorScale` is written. Pinned in Task 3.
3. **A pivot typed as `abc`, `1e400` or empty, and a count of `1`, `0` or `300`**: refused in words under the row, the field stays open, and nothing is written. Pinned in Task 4.
4. **`colorDomain: Fixed` and a later edit that changes colour's column**: the rebuild drops the old pin and the domain row reads auto again, since the column change removes `colorDomain`. Pinned in Task 6.
5. **The legend moved below and the colour list reopened**: the band's cells and the rows still read the plot (now `root/vconcat[0]`), and the ring is drawn round the band, not round a stale right-hand rect. Pinned in Task 8.

---

### Task 1: The ramp's low end is the column's minimum

**Files:**
- Modify: `crates/brightfield-render/src/mark.rs` (`DotRenderer::augment_fill_ramp`, around line 855, and its doc comment)
- Modify: `crates/brightfield-render/src/mark.rs` (`CellRenderer::augment_scales`, if it anchors at zero the same way; read it first)
- Test: `crates/brightfield-render/src/mark.rs` (`mod tests`)
- Baselines: every snapshot under `crates/brightfield-shell/tests/snapshots/` that the change moves

**Interfaces:**
- Produces: a number fill's `Scale::Sequential` runs `[min, max]` of the rows drawn, whatever their sign.

- [ ] **Step 1: Write the failing test**

In `mark.rs`'s `mod tests`, beside the existing `augment_fill_ramp` tests (find one with `rg -n "augment_fill_ramp" crates/brightfield-render/src/mark.rs`), add:

```rust
/// A ramp's low end is the column's smallest value, as Mosaic draws it, and
/// not zero: a column of 2 to 10 runs its ramp from 2.
#[test]
fn a_non_negative_fill_ramp_starts_at_the_columns_minimum() {
    let batch = number_batch("v", &[2.0, 6.0, 10.0]);
    let mut channel_map = ChannelMap::new();
    channel_map.set(Channel::Fill, "v");
    let mut scales = ScaleSet::new();
    DotRenderer::augment_fill_ramp(
        SequentialScheme::Viridis,
        ColourScale::Linear,
        &mut scales,
        &batch,
        &channel_map,
    );
    match scales.get(Channel::Fill) {
        Some(Scale::Sequential { domain_min, domain_max, .. }) => {
            assert_eq!((*domain_min, *domain_max), (2.0, 10.0));
        }
        other => panic!("a number fill builds a ramp, got {other:?}"),
    }
}
```

`number_batch` and the `ChannelMap` setter: reuse whatever the neighbouring tests use (`rg -n "fn .*batch" crates/brightfield-render/src/mark.rs`). Use their names exactly.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo +1.95.0 test -p brightfield-render a_non_negative_fill_ramp_starts_at_the_columns_minimum`
Expected: FAIL, left `(0.0, 10.0)`.

- [ ] **Step 3: Drop the zero anchor**

Replace

```rust
        let (d0, d1) = if lo >= 0.0 { (0.0, hi) } else { (lo, hi) };
```

with

```rust
        // The rows' own ends, as Mosaic draws a ramp: a legend whose low end is
        // zero for a column that never reaches it spends part of its colours on
        // values no row holds.
        let (d0, d1) = (lo, hi);
```

Rewrite the doc comment's "anchored `[0, max]` when the column's minimum is not negative and `[min, max]` otherwise" as "running `[min, max]` of the rows drawn". Do the same in `CellRenderer::augment_scales` if it carries the same anchor, with its own test.

- [ ] **Step 4: Run the render crate's tests**

Run: `cargo +1.95.0 test -p brightfield-render`
Expected: PASS, apart from tests that asserted the zero anchor. Read each one. Where it asserted `0.0` as a low end for a non-negative column, change the expectation to the column's minimum and rename the test if its name states the old rule.

- [ ] **Step 5: Redraw the baselines that move**

Run: `cargo +1.95.0 test -p brightfield-shell --no-fail-fast -- --test-threads=1 2>&1 | rg "snapshot|FAILED"` to list the snapshot tests that fail. For each file listed, run `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test <file>`. Open each changed PNG beside its old version (`git diff --stat`, then `git show HEAD:<path> > /tmp/old.png` and Read both). Confirm the only change is the ramp's colours and the legend's low-end label. Keep a list, file by file, for the PR.

- [ ] **Step 6: Gate and commit**

Run: `cargo xtask ci`
Expected: all steps pass.

```bash
git add crates/brightfield-render/src/mark.rs crates/brightfield-shell/tests/snapshots
git commit -m "Start a number fill's ramp at the column's minimum, not zero"
```

---

### Task 2: Brightfield's own diverging pair is recognised in a colorRange

**Files:**
- Modify: `crates/brightfield-render/src/scale.rs` (near `diverging_stops`, line ~1668, and `apply_colour_override`, line ~1405)
- Test: `crates/brightfield-render/src/scale.rs` (`mod tests`)

**Interfaces:**
- Produces: `pub fn own_diverging_range() -> [[f32; 4]; 3]`, the light theme's three stops (blue pole, light middle, red pole), and `pub fn is_own_diverging_range(range: &[[f32; 4]]) -> bool`. Task 3's writer uses the first to write the file. `apply_colour_override` uses the second.

- [ ] **Step 1: Write the failing test**

```rust
/// A file that names brightfield's own diverging pair draws the design
/// system's ramp in the theme it is drawn in: the dark middle in dark mode,
/// though the file holds the light one.
#[test]
fn the_own_diverging_pair_draws_the_themes_middle() {
    let dark = ChartInk::for_mode(true);
    let mut set = ScaleSet::in_ink(dark.clone());
    set.insert(
        Channel::Fill,
        Scale::Diverging {
            domain_min: -1.0,
            domain_max: 1.0,
            pivot: 0.0,
            stops: diverging_stops(None, &dark),
        },
    );
    let ov = ColourOverride {
        range: Some(own_diverging_range().to_vec()),
        ..ColourOverride::default()
    };
    apply_colour_override(&mut set, &ov);
    let Some(Scale::Diverging { stops, .. }) = set.get(Channel::Fill) else {
        panic!("the fill stays diverging");
    };
    assert_eq!(stops, &diverging_stops(None, &dark));
    assert_eq!(stops[stops.len() / 2], dark.diverging_mid);
}
```

Check `ChartInk`'s constructor and field names (`rg -n "pub fn for_mode|pub diverging_mid" crates/brightfield-render/src/ink.rs`) and adjust to them.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo +1.95.0 test -p brightfield-render the_own_diverging_pair_draws_the_themes_middle`
Expected: FAIL to compile (`own_diverging_range` not found).

- [ ] **Step 3: Implement**

Beside `diverging_stops`:

```rust
/// brightfield's diverging pair as a file writes it: the blue pole, the light
/// theme's middle and the red pole. Mosaic has no scheme of that name, so the
/// file carries the colours, and Mosaic on a white page draws what brightfield
/// draws in light mode.
#[must_use]
pub fn own_diverging_range() -> [[f32; 4]; 3] {
    [
        DIVERGING_BLUE_ARM[0],
        crate::ink::components(meridian_design::viz::DIVERGING_MID_LIGHT),
        DIVERGING_RED_ARM[DIVERGING_RED_ARM.len() - 1],
    ]
}

/// Whether `range` is brightfield's own pair, which is drawn as the design
/// system's ramp in the theme the chart is drawn in rather than as the three
/// colours written.
#[must_use]
pub fn is_own_diverging_range(range: &[[f32; 4]]) -> bool {
    const CLOSE: f32 = 1.0 / 512.0;
    let own = own_diverging_range();
    range.len() == own.len()
        && range
            .iter()
            .zip(own.iter())
            .all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= CLOSE))
}
```

The tolerance absorbs the round trip through an eight-bit hex in the file. Check `crate::ink::components` is `const`-callable outside a `const` (it is used in a `const` at line ~1630, so a plain call works too).

In `apply_colour_override`, in the `Scale::Diverging` arm, change:

```rust
                let stops = match &ov.range {
                    Some(r) if r.len() >= 2 => r.clone(),
                    _ => stops.clone(),
                };
```

to

```rust
                // brightfield's own pair keeps the ramp the scale was built
                // with, which is the theme's: the file's three colours are the
                // light theme's.
                let stops = match &ov.range {
                    Some(r) if is_own_diverging_range(r) => stops.clone(),
                    Some(r) if r.len() >= 2 => r.clone(),
                    _ => stops.clone(),
                };
```

- [ ] **Step 4: Run the tests**

Run: `cargo +1.95.0 test -p brightfield-render`, then `cargo +1.95.0 test -p brightfield-shell --test colour_vocabulary_diverging --test colour_vocabulary_domain`
Expected: PASS. If a shell test wrote the light triple as `colorRange` and asserted the three colours were drawn verbatim, it now gets the full ramp. Read it, and change the expectation only if the test's subject is a hand-written range. If so, give that test a range that is not the pair.

- [ ] **Step 5: Gate and commit**

```bash
cargo xtask ci
git add crates/brightfield-render/src/scale.rs crates/brightfield-shell/tests
git commit -m "Draw brightfield's own diverging pair in the current theme's ramp"
```

---

### Task 3: The colour scale row: model and writer

**Files:**
- Modify: `crates/brightfield-shell/src/shelf.rs`: row constants (~line 708-900), `SettingValue` (~1086), `row_key` (1124), `row_default` (1162), `SettingRow` (~1280), `SettingRow::steps`/`stepped` (1196-1270), `ChannelSettings::of_plot_*`/`build` (1350-1420), `colour_rows` (1441)
- Modify: `crates/brightfield-shell/src/shelf_edit.rs`: new `put_colour_scale`
- Modify: `crates/brightfield-shell/src/app.rs`: `axis_row_written` (2285)
- Modify: `crates/brightfield-shell/src/window.rs`: `hero_axis_settings` (10109) and its callers
- Test: `crates/brightfield-shell/tests/shelf_settings_colour.rs`, `crates/brightfield-shell/tests/shelf_edit_written.rs`

**Interfaces:**
- Consumes: `own_diverging_range()` (Task 2); `brightfield_engine::profile::ColumnMoments { min, max, median, .. }`; `resolve_colour_scale_diverging`, `resolve_colour_scale_quantize`, `resolve_colour_pivot`, `resolve_colour_steps`, `DEFAULT_COLOUR_STEPS`, `MAX_COLOUR_STEPS` from `brightfield_spec::layout`.
- Produces:
  - `pub const PIVOT_ROW: &str = "pivot"`, `pub const STEPS_ROW: &str = "steps"`, `pub const COLOUR_SCALE_STEPS: [&str; 3] = ["linear", "diverging", "steps"]`.
  - `pub struct Number(f64)`: finite only, with `Number::new(f64) -> Option<Number>`, `get(self) -> f64` and `impl Eq`.
  - `SettingValue::ColourScale(ColourScaleWrite)` and `SettingValue::Number(Number)`.
  - `pub enum ColourScaleWrite { Linear, Diverging { pivot: Option<Number> }, Steps { count: usize } }`.
  - `pub struct ColourScaleFacts { pub pivot: Option<f64>, pub auto_pivot: Option<f64>, pub count: Option<usize> }`, as a new `SettingRow` field `pub colour: Option<ColourScaleFacts>`.
  - `ChannelSettings::of_plot_profiled(spec, plot, channels, drawn, marks, moments: Option<&ColumnMoments>) -> Self`.
  - `pub fn auto_pivot(moments: &ColumnMoments) -> f64`.
  - `shelf_edit::put_colour_scale(spec: &mut Spec, plot: &ComponentPath, write: &ColourScaleWrite) -> Result<Vec<ChartEdit>, ShelfRefusal>`.

- [ ] **Step 1: Write the failing row tests**

In `tests/shelf_settings_colour.rs`, the helper `settings_over` builds settings from a plot's attributes. Add a profiled variant and four tests:

```rust
/// Moments of a column of 0.5 to 15 whose median is 3.47: no zero crossing,
/// so the auto pivot is the median.
fn income_moments() -> ColumnMoments {
    ColumnMoments {
        mean: 3.87,
        median: 3.47,
        sd: Some(1.9),
        distinct: 12_000,
        min: 0.5,
        max: 15.0,
        distribution: Distribution::Bins(vec![0; 480]),
    }
}

fn colour_row(attrs: &str, name: &str, drawn: &ScaleSet) -> SettingRow {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml).expect("the spec parses").spec;
    let plot = plot_at_path(&spec, "root").expect("the root is its plot");
    let moments = income_moments();
    ChannelSettings::of_plot_profiled(
        &spec, plot, &channels(), drawn, &[MarkKind::Dot], Some(&moments),
    )
    .rows(ShelfChannel::Colour)
    .iter()
    .find(|r| r.name == name)
    .unwrap_or_else(|| panic!("colour has no {name} row"))
    .clone()
}

#[test]
fn colours_head_rows_are_scheme_scale_legend_and_reverse() {
    let settings = settings_over("", &ramp_scales(), Some(&[MarkKind::Dot]));
    let head: Vec<&str> = settings
        .rows(ShelfChannel::Colour)
        .iter()
        .filter(|r| !r.by_name)
        .map(|r| r.name)
        .collect();
    assert_eq!(head, ["scheme", "scale", "legend", "reverse"]);
}

#[test]
fn the_scale_row_reads_its_value_and_carries_its_parameter() {
    let linear = colour_row("", SCALE_ROW, &ramp_scales());
    assert_eq!((linear.value.as_str(), linear.set), ("linear", false));

    let auto = colour_row("colorScale: diverging\ncolorPivot: 3.47", SCALE_ROW, &ramp_scales());
    assert_eq!(auto.value, "diverging");
    assert_eq!(auto.colour.as_ref().and_then(|c| c.pivot), Some(3.47));
    assert!(auto.set, "a scale off linear is set");
    assert!(!auto.parameter_set(), "the pivot equal to the auto pivot reads auto");

    let typed = colour_row("colorScale: diverging\ncolorPivot: 2", SCALE_ROW, &ramp_scales());
    assert!(typed.parameter_set(), "a pivot off the auto pivot is set");

    let steps = colour_row("colorScale: quantize\ncolorN: 5", SCALE_ROW, &ramp_scales());
    assert_eq!(steps.value, "steps");
    assert_eq!(steps.colour.as_ref().and_then(|c| c.count), Some(5));
    assert!(!steps.parameter_set());
}

/// A file written by hand with diverging and no pivot reads the pivot the
/// chart drew, muted: nothing in the file is the analyst's beyond the scale.
#[test]
fn a_diverging_scale_with_no_pivot_written_reads_the_drawn_pivot_as_auto() {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::Fill,
        Scale::Diverging { domain_min: 0.0, domain_max: 7.0, pivot: 3.5, stops: vec![[0.0; 4]; 3] },
    );
    let row = colour_row("colorScale: diverging", SCALE_ROW, &drawn);
    assert_eq!(row.colour.as_ref().and_then(|c| c.pivot), Some(3.5));
    assert!(!row.parameter_set());
}

#[test]
fn the_scale_row_is_muted_on_a_colour_of_names_and_does_not_step() {
    let row = colour_row("", SCALE_ROW, &names_scales());
    assert!(row.reason.is_some());
    assert_eq!(row.stepped(1), None);
}

#[test]
fn stepping_to_diverging_carries_the_auto_pivot() {
    let row = colour_row("", SCALE_ROW, &ramp_scales());
    assert_eq!(
        row.stepped(1),
        Some(SettingValue::ColourScale(ColourScaleWrite::Diverging {
            pivot: Number::new(3.47),
        })),
    );
    let diverging = colour_row("colorScale: diverging\ncolorPivot: 3.47", SCALE_ROW, &ramp_scales());
    assert_eq!(
        diverging.stepped(1),
        Some(SettingValue::ColourScale(ColourScaleWrite::Steps { count: 5 })),
    );
    assert_eq!(
        diverging.stepped(-1),
        Some(SettingValue::ColourScale(ColourScaleWrite::Linear)),
    );
}

#[test]
fn pivot_and_steps_are_found_by_name_and_muted_off_their_scale() {
    let pivot = colour_row("", PIVOT_ROW, &ramp_scales());
    assert!(pivot.by_name);
    assert_eq!(pivot.reason.as_deref(), Some("pivot applies to a diverging scale"));
    let steps = colour_row("", STEPS_ROW, &ramp_scales());
    assert_eq!(steps.reason.as_deref(), Some("steps applies to a stepped scale"));
    let on = colour_row("colorScale: quantize\ncolorN: 7", STEPS_ROW, &ramp_scales());
    assert_eq!((on.value.as_str(), on.set, on.reason.clone()), ("7", true, None));
}
```

Add the new names to the file's `use brightfield_shell::shelf::{..}` list, and `use brightfield_engine::profile::{ColumnMoments, Distribution};`. Check the module path with `rg -n "pub use|pub mod profile" crates/brightfield-engine/src/lib.rs`. The existing head-rows test (`tab_on_colour_shows_scheme_then_reverse_each_auto_with_the_foot` in `shelf_settings_colour_window.rs`) now fails, and Step 6 updates it.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour`
Expected: FAIL to compile on the new names.

- [ ] **Step 3: Add the row model**

In `shelf.rs`, beside `SCHEME_ROW`:

```rust
/// The name of colour's pivot row, found by typing it.
pub const PIVOT_ROW: &str = "pivot";

/// The name of colour's steps row, found by typing it.
pub const STEPS_ROW: &str = "steps";

/// The values colour's scale row steps through, in the order `l` steps them.
/// `steps` is Mosaic's `quantize`.
pub const COLOUR_SCALE_STEPS: [&str; 3] = ["linear", "diverging", "steps"];

const COLOUR_SCALE_SAYS: &str = "How values map onto colours: along one ramp, \
    diverging either side of a pivot, or cut into steps. Auto is linear.";
const PIVOT_SAYS: &str = "The value at the middle of a diverging scale. Auto is \
    0 where the column crosses it, else the column's median.";
const STEPS_SAYS: &str = "How many colours a stepped scale cuts the values into. Auto is 5.";
const PIVOT_OFF: &str = "pivot applies to a diverging scale";
const STEPS_OFF: &str = "steps applies to a stepped scale";
const SCALE_NAMES: &str = "a colour of names has no scale to choose";
```

Copy the voice of `SCHEME_SAYS` and `SCHEME_NAMES` for the sentences. If `SCHEME_NAMES` already says what `SCALE_NAMES` would, reuse it.

The number type and the write, beside `Ends`:

```rust
/// A finite number a row writes: a pivot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number(f64);

// `Number::new` admits finite numbers alone, so `==` is reflexive.
impl Eq for Number {}

impl Number {
    /// `n`, where it is finite.
    #[must_use]
    pub fn new(n: f64) -> Option<Self> {
        n.is_finite().then_some(Self(n))
    }

    /// The number.
    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

/// What a step on colour's scale row writes: the scale, with the parameter it
/// carries named in the file, since Mosaic's own pivot is 0 and its steps are
/// rounded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColourScaleWrite {
    /// One ramp: `colorScale`, `colorPivot` and `colorN` come out.
    Linear,
    /// Diverging about `pivot`. `None` where no auto pivot is known, and then
    /// none is written and the chart picks its own.
    Diverging { pivot: Option<Number> },
    /// Stepped in `count` colours.
    Steps { count: usize },
}
```

Add `ColourScale(ColourScaleWrite)` and `Number(Number)` to `SettingValue`, each with a one-line doc.

The facts and the new `SettingRow` field:

```rust
/// What only colour's scale, pivot and steps rows know.
#[derive(Clone, Debug, PartialEq)]
pub struct ColourScaleFacts {
    /// The pivot the file writes, else the one the chart drew. `None` off diverging.
    pub pivot: Option<f64>,
    /// brightfield's auto pivot for the column, from its load-time profile, or
    /// the drawn pivot where the file writes none. `None` where neither is known.
    pub auto_pivot: Option<f64>,
    /// The count the file writes, else the default. `None` off steps.
    pub count: Option<usize>,
}

// Pivots are finite where present, so `==` is reflexive.
impl Eq for ColourScaleFacts {}
```

Add `pub colour: Option<ColourScaleFacts>` to `SettingRow`, documented like `range`. Add `colour: None` to every `SettingRow { .. }` literal; `rg -n "SettingRow \{" crates` lists sixteen.

`parameter_set` and the auto pivot:

```rust
impl SettingRow {
    /// Whether the scale row's pivot or count is the analyst's: it differs from
    /// brightfield's own. Compared at the precision a person types, so a median
    /// written back to the file reads auto.
    #[must_use]
    pub fn parameter_set(&self) -> bool {
        let Some(facts) = &self.colour else { return false };
        match (self.value.as_str(), facts.pivot, facts.auto_pivot, facts.count) {
            ("diverging", Some(pivot), Some(auto), _) => {
                (pivot - auto).abs() > 1e-9 * auto.abs().max(1.0)
            }
            ("steps", _, _, Some(count)) => count != DEFAULT_COLOUR_STEPS,
            _ => false,
        }
    }
}

/// brightfield's auto pivot for a column: 0 where its values cross zero, else
/// its median. Read from the whole table's profile, so a sweep on another tile
/// does not move it.
#[must_use]
pub fn auto_pivot(moments: &ColumnMoments) -> f64 {
    if moments.min < 0.0 && moments.max > 0.0 {
        0.0
    } else {
        moments.median
    }
}
```

Stepping. In `SettingRow::steps`, add `COLOUR_SCALE_STEPS.contains(&self.value.as_str()) && self.colour.is_some()` to the `SCALE_ROW` arm. In `stepped`, before the axis `SCALE_ROW` branch:

```rust
        if self.name == SCALE_ROW && self.colour.is_some() {
            let at = COLOUR_SCALE_STEPS.iter().position(|w| *w == self.value)?;
            let next = at.checked_add_signed(by).filter(|n| *n < COLOUR_SCALE_STEPS.len())?;
            let facts = self.colour.as_ref()?;
            return Some(SettingValue::ColourScale(match COLOUR_SCALE_STEPS[next] {
                "diverging" => ColourScaleWrite::Diverging {
                    pivot: facts.auto_pivot.and_then(Number::new),
                },
                "steps" => ColourScaleWrite::Steps { count: DEFAULT_COLOUR_STEPS },
                _ => ColourScaleWrite::Linear,
            }));
        }
```

`ChannelSettings`: give `build` a `moments: Option<&ColumnMoments>` parameter, pass `None` from `of_plot_drawn` and `of_plot_marked`, and add:

```rust
    /// [`Self::of_plot_marked`] with the load-time profile of colour's column,
    /// which the scale row's auto pivot is read from. Without it the auto pivot
    /// is the one the chart drew.
    #[must_use]
    pub fn of_plot_profiled(
        spec: &Spec,
        plot: &PlotNode,
        channels: &ShelfChannels,
        drawn: &ScaleSet,
        marks: &[MarkKind],
        moments: Option<&ColumnMoments>,
    ) -> Self {
        Self::build(spec, plot, channels, drawn, Some(marks), moments)
    }
```

`colour_rows(spec, plot, drawn, marks, moments)` builds, in order: scheme, scale, legend (Task 7 fills it in; until then leave it out and keep the head-rows test red only on `legend`, or mark that one assertion `// legend: Task 7`), reverse, then the by-name rows pivot, steps and domain (domain is Task 5). The scale row:

```rust
    let diverging = resolve_colour_scale_diverging(plot, &spec.params);
    let quantize = resolve_colour_scale_quantize(plot, &spec.params);
    let drawn_pivot = match drawn.get(Channel::Fill) {
        Some(Scale::Diverging { pivot, .. }) => Some(*pivot),
        _ => None,
    };
    let written_pivot = resolve_colour_pivot(plot, &spec.params);
    let facts = ColourScaleFacts {
        pivot: diverging.then(|| written_pivot.or(drawn_pivot)).flatten(),
        auto_pivot: moments.map(auto_pivot).or(drawn_pivot),
        count: quantize.then(|| {
            resolve_colour_steps(plot, &spec.params).unwrap_or(DEFAULT_COLOUR_STEPS)
        }),
    };
    let scale_word = if diverging { "diverging" } else if quantize { "steps" } else { "linear" };
```

The scale `SettingRow` is `name: SCALE_ROW`, `value: scale_word`, `set: scale_word != "linear"`, `kind: Enumerated`, `says: COLOUR_SCALE_SAYS`, `reason: names.then(..SCALE_NAMES)`, `colour: Some(facts.clone())`. The pivot row is `by_name: true`, `kind: Typed`, value `facts.pivot.map(end_text)`, `set: parameter_set` of the scale row, and reason `PIVOT_OFF` unless diverging. The steps row follows the same pattern with `count` and `STEPS_OFF`.

- [ ] **Step 4: Run the row tests**

Run: `cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour`
Expected: the new tests PASS, apart from the legend assertion left for Task 7.

- [ ] **Step 5: Write the failing writer test**

In `tests/shelf_edit_written.rs`, beside the colour-edit test that uses `with_colour`:

```rust
#[test]
fn the_scale_rows_three_values_write_and_take_out_their_keys() {
    let mut opened = open("generated_california_housing.yaml");
    let edits = put_colour(&mut opened.spec, &opened.hero, "median_income", &opened.table)
        .expect("colour takes the column");
    let coloured = write_all(&opened.text, &edits);

    let mut spec = opened.spec.clone();
    let diverging = put_colour_scale(
        &mut spec,
        &opened.hero,
        &ColourScaleWrite::Diverging { pivot: Number::new(3.47) },
    )
    .expect("diverging writes");
    let written = write_all(&coloured, &diverging);
    assert!(written.contains("colorScale: diverging"));
    assert!(written.contains("colorPivot: 3.47"));
    assert!(written.contains("colorRange: ["), "the pair is written as colours");
    assert!(!written.contains("colorScheme"), "diverging takes the scheme out");
    assert_eq!(parse_spec(&written, Format::Yaml).unwrap().spec, spec);

    let steps = put_colour_scale(&mut spec, &opened.hero, &ColourScaleWrite::Steps { count: 5 })
        .expect("steps writes");
    let written = write_all(&written, &steps);
    assert!(written.contains("colorScale: quantize"));
    assert!(written.contains("colorN: 5"));
    assert!(written.contains("colorScheme: viridis"));
    assert!(!written.contains("colorPivot") && !written.contains("colorRange"));

    let linear = put_colour_scale(&mut spec, &opened.hero, &ColourScaleWrite::Linear)
        .expect("linear writes");
    let written = write_all(&written, &linear);
    assert!(!written.contains("colorScale") && !written.contains("colorN"));
    assert!(written.contains("colorScheme: viridis"), "linear keeps the scheme");
}
```

Use the fixture name and the `open` helper exactly as the file's other tests use them.

- [ ] **Step 6: Implement `put_colour_scale` and route the row to it**

In `shelf_edit.rs`:

```rust
/// Write colour's scale on the plot at `plot`: the keys the scale and its
/// parameter take, and the keys the other two scales leave behind taken out.
///
/// Diverging names its pivot and brightfield's pair, and takes the scheme out,
/// since the pair is the diverging scale's colours. Steps names its count and
/// puts the scheme back where the plot names none, since a saved file names the
/// scheme it is drawn in. Linear takes the scale's keys out and leaves the
/// scheme. A `colorRange` that is not brightfield's pair is the analyst's and
/// stays.
pub fn put_colour_scale(
    spec: &mut Spec,
    plot: &ComponentPath,
    write: &ColourScaleWrite,
) -> Result<Vec<ChartEdit>, ShelfRefusal> {
    let pair = own_pair_value();
    let holds_pair = edit::plot_at_path(spec, &plot.0)
        .and_then(|p| p.attributes.get("colorRange"))
        .is_some_and(|v| *v == pair);
    let set = |key: &str, value: SpecValue| ChartEdit::SetPlotAttribute {
        plot: plot.clone(),
        key: key.to_string(),
        value,
    };
    let remove = |key: &str| ChartEdit::RemovePlotAttribute {
        plot: plot.clone(),
        key: key.to_string(),
    };
    let mut wanted = Vec::new();
    match write {
        ColourScaleWrite::Linear => {
            wanted.extend([remove("colorScale"), remove("colorPivot"), remove("colorN")]);
            if holds_pair {
                wanted.push(remove("colorRange"));
                wanted.push(set(SCHEME_KEY, SpecValue::String(SequentialScheme::default().wire_name().into())));
            }
        }
        ColourScaleWrite::Diverging { pivot } => {
            wanted.push(set("colorScale", SpecValue::String("diverging".into())));
            match pivot {
                Some(p) => wanted.push(set("colorPivot", number_value(p.get()))),
                None => wanted.push(remove("colorPivot")),
            }
            wanted.extend([remove("colorN"), remove(SCHEME_KEY), set("colorRange", pair.clone())]);
        }
        ColourScaleWrite::Steps { count } => {
            wanted.push(set("colorScale", SpecValue::String("quantize".into())));
            wanted.push(set("colorN", SpecValue::Integer(i64::try_from(*count).unwrap_or(5))));
            wanted.push(remove("colorPivot"));
            if holds_pair {
                wanted.push(remove("colorRange"));
            }
            let names_scheme = edit::plot_at_path(spec, &plot.0)
                .is_some_and(|p| p.attributes.contains_key(SCHEME_KEY));
            if !names_scheme || holds_pair {
                wanted.push(set(SCHEME_KEY, SpecValue::String(SequentialScheme::default().wire_name().into())));
            }
        }
    }
    apply_present(spec, plot, wanted)
}

/// brightfield's diverging pair as the file holds it: three hex strings.
fn own_pair_value() -> SpecValue {
    SpecValue::Array(
        brightfield_render::scale::own_diverging_range()
            .iter()
            .map(|c| SpecValue::String(hex(*c)))
            .collect(),
    )
}

/// `#rrggbb` for a colour, alpha dropped.
fn hex(c: [f32; 4]) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}
```

`apply_present` applies each edit with `edit::apply_for_fresh_load`. It keeps only the edits that change the spec: a removal of a key the plot lacks is dropped, and so is a set to the value already held. It returns the kept edits. If `shelf_edit.rs` already has such a helper (read `put_colour` at line 416 for how it applies and collects), use it; otherwise write it there with a doc comment.

Check the file's colour-literal parser reads lower-case `#rrggbb` (`rg -n "fn parse_colour_literal" crates/brightfield-render/src/mark.rs`).

In `app.rs` `axis_row_written`, before the `row_key` lookup:

```rust
        if let SettingValue::ColourScale(write) = &edit.value {
            let mut spec = before.clone();
            let edits = crate::shelf_edit::put_colour_scale(&mut spec, path, write)?;
            return Ok((!edits.is_empty()).then_some((spec, edits)));
        }
        if edit.channel == ShelfChannel::Colour
            && edit.row == crate::shelf::SCALE_ROW
            && edit.value == SettingValue::Auto
        {
            let mut spec = before.clone();
            let edits = crate::shelf_edit::put_colour_scale(
                &mut spec,
                path,
                &crate::shelf::ColourScaleWrite::Linear,
            )?;
            return Ok((!edits.is_empty()).then_some((spec, edits)));
        }
```

Add to `row_key`'s colour arm: `PIVOT_ROW => Some("colorPivot")` and `STEPS_ROW => Some("colorN")`. Add to `row_default`: `PIVOT_ROW => SpecValue::Null` (a pivot is always written by name, as the scheme is) and `STEPS_ROW => SpecValue::Integer(5)`. In `axis_row_written`'s value match, add `SettingValue::Number(n) => SettingWrite::Value(number_value(n.get()))`, and make `SettingValue::ColourScale(_)` unreachable there by the early return above.

In `window.rs`, give `hero_axis_settings` the table profiles and use `of_plot_profiled`:

```rust
fn hero_axis_settings(
    doc: &ChartDoc,
    channels: &ShelfChannels,
    table: &[brightfield_engine::ColumnProfile],
) -> Option<ChannelSettings> {
    let hero = doc.composed.plots.get(HERO_PLOT)?;
    let spec = doc.live_dashboard()?.spec();
    let plot = brightfield_spec::edit::plot_at_path(spec, &hero.path)?;
    let moments = match &channels.colour {
        Binding::Column(name) => table
            .iter()
            .find(|p| &p.name == name)
            .and_then(|p| p.moments.as_ref()),
        _ => None,
    };
    Some(ChannelSettings::of_plot_profiled(
        spec, plot, channels, &hero.scales, &hero.marks, moments,
    ))
}
```

Each caller (`rg -n "hero_axis_settings\(" crates/brightfield-shell/src/window.rs`) passes `self.protocol.doc.model.source().map_or(&[], |s| s.profiles.as_slice())`, as `shelf_apply` reads it at line ~5634.

- [ ] **Step 7: Window test: stepping the scale row writes and redraws**

In `tests/shelf_settings_colour_window.rs`, update `tab_on_colour_shows_scheme_then_reverse_each_auto_with_the_foot` to expect `["scheme", "scale", "legend", "reverse"]`, or `["scheme", "scale", "reverse"]` until Task 7. Then add:

```rust
#[test]
fn stepping_the_scale_row_to_diverging_writes_it_with_the_auto_pivot() {
    let mut w = Window::housing("scale_diverging");
    w.open_cell(Key::C, "c");
    w.keep_on_colour("median_income");
    w.open_colour_settings();
    w.move_to_row(SCALE_ROW);
    w.press(Key::L);
    let row = w.row(SCALE_ROW);
    assert_eq!(row.value, "diverging");
    assert!(!row.parameter_set(), "the pivot written is the auto pivot");
    let text = w.spec_text();
    assert!(text.contains("colorScale: diverging"));
    assert!(text.contains("colorPivot: "));
    assert!(!text.contains("colorScheme"));
}
```

`move_to_row` presses `j` until `setting_cursor()` names the row. `spec_text` reads the text Save would write. Write either helper only if the file has no equivalent; `rg -n "fn " crates/brightfield-shell/tests/shelf_settings_colour_window.rs` lists what exists.

- [ ] **Step 8: Run, gate, commit**

Run: `cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour --test shelf_settings_colour_window --test shelf_edit_written`, then `cargo xtask ci`.
Expected: PASS.

```bash
git add crates/brightfield-shell crates/brightfield-render
git commit -m "Give colour's settings a scale row: linear, diverging about a pivot, steps"
```

---

### Task 4: The scale row's parameter field and its drawing

**Files:**
- Modify: `crates/brightfield-shell/src/shelf.rs`: `SettingRow::takes_field` (1208), `ColumnList::open_field` (2289), `field_edit` (2346), the row's paint (find where a `SettingRow`'s value text is laid out, `rg -n "fn paint_row|fn draw_row|SettingRowDrawn" crates/brightfield-shell/src/shelf.rs`)
- Test: `crates/brightfield-shell/tests/shelf_settings_colour.rs`, `crates/brightfield-shell/tests/shelf_settings_colour_window.rs`
- Baselines: three new snapshot pairs

**Interfaces:**
- Consumes: `SettingRow.colour`, `parameter_set()`, `PIVOT_ROW`, `STEPS_ROW`, `SettingValue::Number`, `SettingValue::Count`, `MAX_COLOUR_STEPS` (Task 3).
- Produces: a field on the scale row whose kept value is `RowEdit { row: PIVOT_ROW, value: Number(..) }` or `RowEdit { row: STEPS_ROW, value: Count(..) }`.

- [ ] **Step 1: Write the failing field tests**

```rust
#[test]
fn enter_on_diverging_opens_the_pivot_and_keeps_a_typed_one() {
    let mut list = turned("colorScale: diverging\ncolorPivot: 3.47");
    list.move_to(SCALE_ROW);
    let reports = list.key(Key::Enter);
    assert!(reports.is_empty());
    assert_eq!(list.field().map(|f| f.text.as_str()), Some("3.47"));
    list.type_text("2");
    let reports = list.key(Key::Enter);
    assert!(reports.contains(&ListReport::Set(RowEdit {
        channel: ShelfChannel::Colour,
        row: PIVOT_ROW,
        value: SettingValue::Number(Number::new(2.0).unwrap()),
    })));
}

#[test]
fn a_pivot_or_count_the_row_cannot_take_is_refused_and_the_field_stays() {
    for (attrs, typed) in [
        ("colorScale: diverging\ncolorPivot: 1", "abc"),
        ("colorScale: diverging\ncolorPivot: 1", "1e400"),
        ("colorScale: quantize\ncolorN: 5", "1"),
        ("colorScale: quantize\ncolorN: 5", "0"),
        ("colorScale: quantize\ncolorN: 5", "300"),
    ] {
        let mut list = turned(attrs);
        list.move_to(SCALE_ROW);
        list.key(Key::Enter);
        list.type_text(typed);
        let reports = list.key(Key::Enter);
        assert!(
            !reports.iter().any(|r| matches!(r, ListReport::Set(_))),
            "{typed} on {attrs} writes nothing",
        );
        assert!(list.field().and_then(|f| f.refusal.clone()).is_some(), "{typed} is refused in words");
    }
}

#[test]
fn enter_on_linear_opens_no_field() {
    let mut list = turned("");
    list.move_to(SCALE_ROW);
    list.key(Key::Enter);
    assert!(list.field().is_none());
}
```

`turned`, `move_to`, `key`, `type_text` and `field`: use the file's own helpers for a list turned to settings and driven by keys (see `turned("colorScheme: blues")` in `fn baseline`). Where a helper is missing, write it against `ColumnList`'s public API in the same style.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour enter_on_ a_pivot_or_count`
Expected: FAIL (no field opens on the scale row).

- [ ] **Step 3: Implement the field**

`takes_field`:

```rust
    pub fn takes_field(&self) -> bool {
        self.reason.is_none()
            && (self.kind == SettingKind::Typed
                || self.name == FORMAT_ROW
                || (self.name == SCALE_ROW
                    && matches!(self.value.as_str(), "diverging" | "steps")
                    && self.colour.is_some()))
    }
```

`open_field`: before the generic text branch, a colour scale row opens on its parameter:

```rust
        if row.name == SCALE_ROW {
            if let Some(facts) = &row.colour {
                let text = match row.value.as_str() {
                    "diverging" => facts.pivot.map(end_text).unwrap_or_default(),
                    _ => facts.count.map(|n| n.to_string()).unwrap_or_default(),
                };
                self.field = Some(RowField {
                    row: if row.value == "diverging" { PIVOT_ROW } else { STEPS_ROW },
                    selected: !text.is_empty(),
                    text,
                    refusal: None,
                    ends: None,
                });
                return;
            }
        }
```

The field's `row` names the parameter, so a typed pivot keeps through the same `field_edit` path as the pivot row found by name. `field_edit` gains two arms before the catch-all:

```rust
            PIVOT_ROW => {
                if field.text.trim().is_empty() {
                    return Ok(None);
                }
                let n = field
                    .text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .and_then(Number::new)
                    .ok_or_else(|| format!("a pivot is a number, and {} is not one", field.text.trim()))?;
                SettingValue::Number(n)
            }
            STEPS_ROW => {
                if field.text.trim().is_empty() {
                    return Ok(None);
                }
                let n: usize = field.text.trim().parse().map_err(|_| {
                    format!("steps is a whole number, and {} is not one", field.text.trim())
                })?;
                if !(2..=MAX_COLOUR_STEPS).contains(&n) {
                    return Err(format!("steps is 2 to {MAX_COLOUR_STEPS}, and {n} is not"));
                }
                SettingValue::Count(n)
            }
```

Check how the field's refusal reaches `RowField::refusal` on Enter: `keep_field` (2478) calls `field_edit`. Confirm it stores the `Err` sentence there, and reuse that path.

- [ ] **Step 4: Paint the parameter behind the word**

Where the row's value text is laid out, for a row with `colour: Some(facts)` on diverging or steps, lay out the parameter after the word: ` about 3.47` on diverging, ` · 5` on steps. Use muted ink unless `row.parameter_set()`, and the row's own ink and dot otherwise. Follow how the range row lays out its drawn ends in muted ink (find `RangeFacts` uses in the paint code). The open field takes the parameter's place, as the format row's field takes its specifier's.

Add `SettingRowDrawn` readings if the tests read the text through it (see how `tests/shelf_settings_colour.rs` reads a row's galleys), and a test:

```rust
#[test]
fn the_scale_row_draws_its_parameter_muted_while_auto() {
    let drawn = draw_list(turned("colorScale: diverging\ncolorPivot: 3.47"), Mode::Light);
    let row = drawn.row(SCALE_ROW);
    assert_eq!(row.text(), "diverging about 3.47");
    assert_eq!(row.ink_of("3.47"), muted_ink(Mode::Light));
    let drawn = draw_list(turned("colorScale: diverging\ncolorPivot: 2"), Mode::Light);
    assert_ne!(drawn.row(SCALE_ROW).ink_of("2"), muted_ink(Mode::Light));
}
```

Use the file's existing frame-reading helpers (`text_ink::frame_text`, `DrawnText`) for `text()` and `ink_of`.

- [ ] **Step 5: Window test: typing the pivot previews, Enter keeps**

```rust
#[test]
fn a_typed_pivot_previews_and_enter_keeps_it_with_its_dot() {
    let mut w = Window::housing("pivot_typed");
    w.open_cell(Key::C, "c");
    w.keep_on_colour("median_income");
    w.open_colour_settings();
    w.move_to_row(SCALE_ROW);
    w.press(Key::L);
    w.press(Key::Enter);
    w.type_letter(Key::Num2, "2");
    assert!(w.app.chart_doc().is_previewing(), "the typed pivot is drawn before it is kept");
    w.press(Key::Enter);
    assert!(w.row(SCALE_ROW).parameter_set());
    assert!(w.spec_text().contains("colorPivot: 2"));
}
```

Replace `is_previewing` with whatever the window exposes for a live shelf preview (`rg -n "shelf_preview" crates/brightfield-shell/src/app.rs`). If nothing is public, read the chart's colour scale pivot from `composed_plot_scales()` or an equivalent accessor instead.

- [ ] **Step 6: The scheme strip on diverging**

On diverging, the scheme row offers brightfield's pair (its auto, as the file holds it: no `colorScheme` and the pair's `colorRange`) and Mosaic's `rdbu`. On linear and steps it offers the four sequential schemes, as now.

Test in `shelf_settings_colour.rs`:

```rust
#[test]
fn on_diverging_the_scheme_row_offers_the_pair_and_rdbu() {
    let pair = format!("colorScale: diverging\ncolorPivot: 1\ncolorRange: {}", PAIR_TEXT);
    let row = colour_row(&pair, SCHEME_ROW, &ramp_scales());
    assert_eq!((row.value.as_str(), row.set), ("blue-red", false));
    assert_eq!(row.stepped(1), Some(SettingValue::Word("rdbu".into())));
    let rdbu = colour_row("colorScale: diverging\ncolorPivot: 1\ncolorScheme: rdbu", SCHEME_ROW, &ramp_scales());
    assert_eq!((rdbu.value.as_str(), rdbu.set), ("rdbu", true));
    assert_eq!(rdbu.stepped(-1), Some(SettingValue::Auto));
    assert_eq!(diverging_strip(), ["blue-red", "rdbu"]);
}
```

`PAIR_TEXT` is the YAML array of the pair's three hex strings, built in the test from `own_diverging_range()` with the same `#rrggbb` rule `shelf_edit::hex` uses.

Implement:
- `pub const OWN_PAIR: &str = "blue-red"` and `pub fn diverging_strip() -> [&'static str; 2] { [OWN_PAIR, "rdbu"] }`.
- In `colour_rows`, on diverging the scheme row reads `rdbu` where `colorScheme: rdbu` is written, and `OWN_PAIR` otherwise; it is set only on `rdbu`.
- `stepped` on the scheme row steps through `diverging_strip()` when the scale row reads diverging (carry that on the row: a `diverging: bool` in `ColourScaleFacts`, read by `stepped`). From `OWN_PAIR`, `l` gives `Word("rdbu")`; from `rdbu`, `h` gives `Auto`.
- In `app.rs` `axis_row_written`, on colour's `SCHEME_ROW` while the plot is diverging (`resolve_colour_scale_diverging`):
  - `Word("rdbu")` writes `colorScheme: rdbu` and takes out a `colorRange` equal to the pair;
  - `Auto` takes `colorScheme` out and writes the pair's `colorRange`.
  
  Put both in a `shelf_edit::put_diverging_scheme(spec, plot, rdbu: bool)` beside `put_colour_scale`, sharing `own_pair_value()`. The viridis rule for `Auto` on the scheme row stays for linear and steps.
- The strip's paint: where the scheme strip lays out its ramps (`rg -n "scheme_strip\(\)" crates/brightfield-shell/src/shelf.rs`), lay out `diverging_strip()` on diverging. Draw the pair's ramp from `diverging_stops(None, &ink)` and `rdbu`'s from `SequentialScheme::Rdbu.stops()`, with the current one ringed as now.

- [ ] **Step 7: Baselines**

Add two snapshot tests beside `colours_scheme_row_with_its_strip_{light,dark}_matches_its_baseline`, following `fn baseline`:
- `colours_scale_row_diverging_{light,dark}`, the cursor on the scale row of `colorScale: diverging\ncolorPivot: 3.47`;
- `colours_scale_row_pivot_field_{light,dark}`, the same with the field open on `2`;
- `colours_scheme_row_diverging_strip_{light,dark}`, the cursor on the scheme row of a diverging plot, showing the pair and `rdbu`.

Generate them with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour`, read each PNG, and check the word, the muted parameter and the field.

- [ ] **Step 8: Run, gate, commit**

```bash
cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour --test shelf_settings_colour_window
cargo xtask ci
git add crates/brightfield-shell
git commit -m "Type a diverging scale's pivot and a stepped scale's count behind the word"
```

---

### Task 5: `colorDomain: Fixed` is read and holds the colours

**Files:**
- Modify: `crates/brightfield-spec/src/layout.rs`: `FixedDomains` (776), `resolve_fixed_domains` (811)
- Modify: `crates/brightfield-render/src/scale.rs`: `PinnedDomains` (1218), `capture` (1268), new `ColourPin`, new `apply_colour_pin`
- Modify: `crates/brightfield-render/src/scene.rs`: `build_multi_mark_scene_pinned` (604)
- Test: `crates/brightfield-spec/src/layout.rs` tests, `crates/brightfield-render/src/scale.rs` tests, a new `crates/brightfield-shell/tests/colour_vocabulary_fixed.rs`

**Interfaces:**
- Produces: `FixedDomains { x, y, colour: bool }`; `pub struct ColourPin { pub lo: f64, pub hi: f64, pub pivot: Option<f64> }`; `PinnedDomains.colour: Option<ColourPin>`; `pub fn apply_colour_pin(scales: &mut ScaleSet, pins: &PinnedDomains)`.

- [ ] **Step 1: Write the failing reader test**

In `layout.rs`'s tests, beside the ones at ~4503:

```rust
#[test]
fn color_domain_fixed_asks_for_a_colour_pin() {
    let plot = plot_with("colorDomain: Fixed");
    assert_eq!(
        resolve_fixed_domains(&plot),
        FixedDomains { x: false, y: false, colour: true }
    );
    assert_eq!(colour_domain(&SpecValue::String("Fixed".into())), None);
}
```

Use the helper the neighbouring tests use to build a `PlotNode` from attributes.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo +1.95.0 test -p brightfield-spec color_domain_fixed_asks_for_a_colour_pin`
Expected: FAIL to compile (no field `colour`).

- [ ] **Step 3: Implement the reader**

```rust
pub struct FixedDomains {
    /// `xDomain: Fixed` is declared on this plot.
    pub x: bool,
    /// `yDomain: Fixed` is declared on this plot.
    pub y: bool,
    /// `colorDomain: Fixed` is declared on this plot: the colour scale holds
    /// the domain of its first draw while filters move the rows.
    pub colour: bool,
}
```

`is_empty` becomes `!self.x && !self.y && !self.colour`, and `resolve_fixed_domains` adds `colour: pinned("colorDomain")`. Fix the other literals (`rg -n "FixedDomains \{" crates`): `scale.rs:4331`, `scale.rs:4382`, `layout.rs:4503`, `layout.rs:4507` take `colour: false`. The corpus golden at `layout.rs:5382` prints `FixedDomains { x: false, y: false }`. Find the file it is read from (`rg -l "FixedDomains \{ x: false, y: false \}" crates`) and regenerate it by the means its test names; every line gains `, colour: false`.

- [ ] **Step 4: Write the failing pin tests**

In `scale.rs`'s tests:

```rust
/// A colour pin holds the ends of the first draw when a later draw has fewer
/// rows: the same value keeps the same colour across a sweep.
#[test]
fn a_colour_pin_holds_the_first_draws_ends() {
    let mut first = ScaleSet::new();
    first.insert(Channel::Fill, Scale::Sequential { domain_min: 0.5, domain_max: 15.0, stops: vec![[0.0; 4], [1.0; 4]] });
    let mut pins = PinnedDomains::default();
    pins.capture(&first, FixedDomains { x: false, y: false, colour: true });
    assert_eq!(pins.colour, Some(ColourPin { lo: 0.5, hi: 15.0, pivot: None }));

    let mut later = ScaleSet::new();
    later.insert(Channel::Fill, Scale::Sequential { domain_min: 0.5, domain_max: 12.5, stops: vec![[0.0; 4], [1.0; 4]] });
    apply_colour_pin(&mut later, &pins);
    assert!(matches!(later.get(Channel::Fill), Some(Scale::Sequential { domain_max, .. }) if *domain_max == 15.0));
}

#[test]
fn a_diverging_colour_pin_holds_the_pivot_too() {
    let mut first = ScaleSet::new();
    first.insert(Channel::Fill, Scale::Diverging { domain_min: -8.0, domain_max: 15.0, pivot: 3.47, stops: vec![[0.0; 4]; 3] });
    let mut pins = PinnedDomains::default();
    pins.capture(&first, FixedDomains { x: false, y: false, colour: true });
    let mut later = ScaleSet::new();
    later.insert(Channel::Fill, Scale::Diverging { domain_min: -6.0, domain_max: 12.5, pivot: 3.2, stops: vec![[0.0; 4]; 3] });
    apply_colour_pin(&mut later, &pins);
    assert!(matches!(
        later.get(Channel::Fill),
        Some(Scale::Diverging { domain_min, domain_max, pivot, .. })
            if (*domain_min, *domain_max, *pivot) == (-8.0, 15.0, 3.47)
    ));
}

#[test]
fn a_colour_pin_leaves_a_scale_of_another_kind_and_a_categorical_colour_pins_nothing() {
    let mut names = ScaleSet::new();
    names.insert(Channel::Fill, Scale::Colour { categories: vec!["a".into()], palette: vec![[0.0; 4]] });
    let mut pins = PinnedDomains::default();
    pins.capture(&names, FixedDomains { x: false, y: false, colour: true });
    assert_eq!(pins.colour, None);

    let pins = PinnedDomains { colour: Some(ColourPin { lo: 0.0, hi: 1.0, pivot: None }), ..PinnedDomains::default() };
    let before = names.clone();
    apply_colour_pin(&mut names, &pins);
    assert_eq!(names, before);
}
```

- [ ] **Step 5: Implement the pin**

```rust
/// A colour scale's pinned domain: the two ends of a ramp, a diverging scale's
/// or a stepped one's, and a diverging scale's pivot, held from the plot's first
/// draw under `colorDomain: Fixed`, so a value keeps its colour while a filter
/// on another tile moves the rows. A categorical colour has no ends, and is not
/// pinned here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourPin {
    /// The low end.
    pub lo: f64,
    /// The high end.
    pub hi: f64,
    /// A diverging scale's pivot. `None` for a ramp or steps.
    pub pivot: Option<f64>,
}

impl ColourPin {
    /// The pin `scale` hands back, or `None` for a scale with no ends.
    #[must_use]
    pub fn of(scale: &Scale) -> Option<Self> {
        match scale {
            Scale::Sequential { domain_min, domain_max, .. }
            | Scale::Quantized { domain_min, domain_max, .. } => {
                Some(Self { lo: *domain_min, hi: *domain_max, pivot: None })
            }
            Scale::Diverging { domain_min, domain_max, pivot, .. } => {
                Some(Self { lo: *domain_min, hi: *domain_max, pivot: Some(*pivot) })
            }
            _ => None,
        }
    }

    /// `scale` re-domained onto this pin, keeping its own colours. `None` where
    /// the scale is not of the pin's kind.
    fn applied_to(&self, scale: &Scale) -> Option<Scale> {
        match (scale, self.pivot) {
            (Scale::Sequential { stops, .. }, None) => Some(Scale::Sequential {
                domain_min: self.lo,
                domain_max: self.hi,
                stops: stops.clone(),
            }),
            (Scale::Quantized { colours, .. }, None) => Some(Scale::Quantized {
                domain_min: self.lo,
                domain_max: self.hi,
                colours: colours.clone(),
            }),
            (Scale::Diverging { stops, .. }, Some(pivot)) => Some(Scale::Diverging {
                domain_min: self.lo,
                domain_max: self.hi,
                pivot,
                stops: stops.clone(),
            }),
            _ => None,
        }
    }
}
```

Add `pub colour: Option<ColourPin>` to `PinnedDomains`, documented as "the colour scale's pin, once captured". `is_empty` includes it. In `capture`, after the loop:

```rust
        if request.colour && self.colour.is_none() {
            self.colour = scales.get(Channel::Fill).and_then(ColourPin::of);
        }
```

And:

```rust
/// Re-domain `scales`' fill onto the colour pin, in place. A fill of another
/// kind from the pin, or no pin, is left as inference made it.
pub fn apply_colour_pin(scales: &mut ScaleSet, pins: &PinnedDomains) {
    let Some(pin) = &pins.colour else { return };
    let Some(scale) = scales.get(Channel::Fill) else { return };
    if let Some(pinned) = pin.applied_to(scale) {
        scales.insert(Channel::Fill, pinned);
    }
}
```

The quantized scale is captured after `apply_colour_override` cut it. The next draw's fill is a `Sequential` until the override cuts it again, so `applied_to` must also take a `Sequential` against a pin captured from `Quantized`. Both are `pivot: None`, so the `Sequential` arm already matches. Keep the `Quantized` arm for a mark that builds steps itself.

In `scene.rs` `build_multi_mark_scene_pinned`, call `apply_colour_pin(&mut scales, &pins);` immediately before `apply_colour_override(&mut scales, colour_override);`, with a comment. A file holds either `Fixed` or two ends under `colorDomain`, never both, so the override's written domain and the pin cannot both act. Running the pin first lets the steps the override cuts divide the pinned domain.

The capture in `pipeline.rs` (~2650, `if !fixed.is_empty()`) now covers colour with no change. Read it to confirm it captures from the scales the scene returned.

- [ ] **Step 6: Shell test across a sweep**

New `crates/brightfield-shell/tests/colour_vocabulary_fixed.rs`. Copy the module doc and fixture style of `tests/colour_vocabulary_domain.rs`. The test loads a two-tile spec where tile A is a dot map with `fill: v` and `colorDomain: Fixed`, filtered by tile B's `intervalX` brush. It presents, commits a brush on B that removes the rows with the largest `v`, presents again, and asserts that A's fill scale ends are those of the first present. A second test, the same spec without `Fixed`, asserts the ends shrink, which pins today's re-fit default. Use the brush-commit helper the committed-brush tests use (`rg -n "fn commit|brush" crates/brightfield-shell/tests/committed_brush_move.rs | head`).

- [ ] **Step 7: Run, gate, commit**

```bash
cargo +1.95.0 test -p brightfield-spec && cargo +1.95.0 test -p brightfield-render && cargo +1.95.0 test -p brightfield-shell --test colour_vocabulary_fixed
cargo xtask ci
git add crates
git commit -m "Hold a colour's domain from its first draw under colorDomain: Fixed"
```

---

### Task 6: The domain row

**Files:**
- Modify: `crates/brightfield-shell/src/shelf.rs`: `DOMAIN_ROW`, `row_key`, `row_default`, `colour_rows`, `SettingRow::steps`/`stepped`, `open_field` and `range_edit` (generalise from `RANGE_ROW` to "a row with `range` facts")
- Modify: `crates/brightfield-shell/src/app.rs`: `axis_row_written` (the domain's `Word("fixed")`)
- Modify: `crates/brightfield-shell/src/shelf_edit.rs`: `put_colour` removes a set domain and re-writes a diverging pivot for the new column
- Test: `crates/brightfield-shell/tests/shelf_settings_colour.rs`, `shelf_settings_colour_window.rs`, `shelf_edit_written.rs`

**Interfaces:**
- Consumes: `RangeFacts`, `Ends`, `end_text`, `ends_text` (existing); `auto_pivot` (Task 3); `resolve_fixed_domains` with `colour` (Task 5); `colour_domain` / `ColourDomain::Ends` from `brightfield_spec::layout`.
- Produces: `pub const DOMAIN_ROW: &str = "domain"`, `pub const FIXED_DOMAIN: &str = "fixed"`, `pub const AUTO_DOMAIN: &str = "auto"`.

- [ ] **Step 1: Write the failing row tests**

```rust
#[test]
fn the_domain_row_reads_auto_with_the_drawn_ends_fixed_or_two_ends() {
    let auto = colour_row("", DOMAIN_ROW, &ramp_scales());
    assert_eq!((auto.value.as_str(), auto.set), ("auto", false));
    assert_eq!(auto.range.as_ref().and_then(|r| r.drawn), Ends::new(0.0, 10.0));
    let fixed = colour_row("colorDomain: Fixed", DOMAIN_ROW, &ramp_scales());
    assert_eq!((fixed.value.as_str(), fixed.set), ("fixed", true));
    let ends = colour_row("colorDomain: [0, 4]", DOMAIN_ROW, &ramp_scales());
    assert_eq!((ends.value.as_str(), ends.set), ("0 \u{2013} 4", true));
    assert!(ends.by_name);
}

#[test]
fn the_domain_row_steps_between_auto_and_fixed() {
    let auto = colour_row("", DOMAIN_ROW, &ramp_scales());
    assert_eq!(auto.stepped(1), Some(SettingValue::Word("fixed".into())));
    let fixed = colour_row("colorDomain: Fixed", DOMAIN_ROW, &ramp_scales());
    assert_eq!(fixed.stepped(-1), Some(SettingValue::Auto));
}

#[test]
fn the_domain_row_is_muted_on_a_colour_of_names() {
    assert!(colour_row("", DOMAIN_ROW, &names_scales()).reason.is_some());
}
```

Check `ends_text`'s separator (`rg -n "fn ends_text" -A6 crates/brightfield-shell/src/shelf.rs`) and match it in the assertion.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour the_domain_row`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the row**

Constants and sentences:

```rust
/// The name of colour's domain row, found by typing it.
pub const DOMAIN_ROW: &str = "domain";
/// The domain row's word for `colorDomain: Fixed`.
pub const FIXED_DOMAIN: &str = "fixed";
const DOMAIN_SAYS: &str = "The two values the colours run between. Auto fits \
    the rows drawn, so a sweep elsewhere re-fits them; fixed holds the first draw's.";
const DOMAIN_NAMES: &str = "a colour of names has no ends to set";
```

In `colour_rows`, read the plot's `colorDomain`: `Fixed` gives `fixed`, `colour_domain(..) == Some(ColourDomain::Ends(lo, hi))` gives `Ends`, anything else gives auto. Take the drawn ends from `drawn.get(Channel::Fill)` (Sequential, Diverging or Quantized `domain_min`/`domain_max`). Build:

```rust
    SettingRow {
        name: DOMAIN_ROW,
        value: match (&fixed, set_ends) {
            (true, _) => FIXED_DOMAIN.to_string(),
            (false, Some(ends)) => ends_text(ends),
            (false, None) => AUTO.to_string(),
        },
        set: fixed || set_ends.is_some(),
        reason: names.then(|| DOMAIN_NAMES.to_string()),
        by_name: true,
        kind: SettingKind::Typed,
        says: DOMAIN_SAYS,
        from: None,
        format: None,
        range: Some(RangeFacts { key: "colorDomain", drawn: drawn_ends, set: set_ends, log: false }),
        colour: None,
    }
```

Paint: the auto value is followed by the drawn ends in muted ink (*auto · 0.50 – 15.00*). Reuse the range row's paint for the ends.

`steps()` adds `DOMAIN_ROW` where `self.value` is `auto` or `fixed`. `stepped()` for `DOMAIN_ROW`: from `auto`, `l` gives `Word("fixed")`; from `fixed`, `h` gives `Auto`; otherwise `None`.

Generalise the range field. In `open_field` and `range_edit`, test `row.range.is_some()` (and `field.ends.is_some()`) instead of `row.name == RANGE_ROW`, and have `range_edit` return `row: field.row`. Keep `RANGE_ROW`'s log rule as it is, since `log` is false on the domain row.

`row_key`'s colour arm: `DOMAIN_ROW => Some("colorDomain")`. `row_default`: `DOMAIN_ROW => SpecValue::Null`. In `app.rs` `axis_row_written`, map `SettingValue::Word(w)` on `DOMAIN_ROW` with `w == FIXED_DOMAIN` to `SettingWrite::Value(SpecValue::String("Fixed".into()))`. The generic `Word` arm would write `fixed` in lower case, which Mosaic does not read. `Ends` already writes the array.

- [ ] **Step 4: A column change resets the domain and the pivot**

Test in `shelf_edit_written.rs`:

```rust
#[test]
fn a_new_column_on_colour_takes_a_set_domain_out_and_re_reads_the_pivot() {
    let mut opened = open("generated_california_housing.yaml");
    put_colour(&mut opened.spec, &opened.hero, "median_income", &opened.table).unwrap();
    put_colour_scale(&mut opened.spec, &opened.hero, &ColourScaleWrite::Diverging { pivot: Number::new(2.0) }).unwrap();
    put_setting(&mut opened.spec, &opened.hero, "colorDomain", &SpecValue::Null,
        &SettingWrite::Value(SpecValue::String("Fixed".into()))).unwrap();

    put_colour(&mut opened.spec, &opened.hero, "house_age", &opened.table).unwrap();
    let plot = plot_at_path(&opened.spec, &opened.hero.0).unwrap();
    assert!(!plot.attributes.contains_key("colorDomain"));
    let age = opened.table.iter().find(|p| p.name == "house_age").and_then(|p| p.moments.as_ref()).unwrap();
    assert_eq!(plot.attributes.get("colorPivot"), Some(&number_value(auto_pivot(age))));
}
```

In `put_colour`, once the fill edits are known to change the column: push `RemovePlotAttribute { key: "colorDomain" }` when the plot holds one. When the plot is diverging (`resolve_colour_scale_diverging`), push `SetPlotAttribute { key: "colorPivot", value: number_value(auto_pivot(moments)) }` using the new column's moments from `table`. Apply both through the same fresh-load path the function uses. A column with no moments takes `colorPivot` out.

- [ ] **Step 5: Window test and baseline**

```rust
#[test]
fn stepping_the_domain_row_to_fixed_writes_colordomain_fixed() {
    let mut w = Window::housing("domain_fixed");
    w.open_cell(Key::C, "c");
    w.keep_on_colour("median_income");
    w.open_colour_settings();
    w.query("dom");
    w.press(Key::L);
    assert_eq!(w.row(DOMAIN_ROW).value, "fixed");
    assert!(w.spec_text().contains("colorDomain: Fixed"));
}
```

`query` types `/` and the letters, as the by-name tests do (`tests/shelf_settings_by_name.rs`). Add a baseline `colours_domain_row_{light,dark}` with the cursor on the domain row at auto, showing the muted drawn ends.

- [ ] **Step 6: Run, gate, commit**

```bash
cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour --test shelf_settings_colour_window --test shelf_edit_written
cargo xtask ci
git add crates/brightfield-shell
git commit -m "Give colour a domain row: auto, fixed, or two ends"
```

---

### Task 7: The legend row

**Files:**
- Modify: `crates/brightfield-spec/src/edit.rs`: new `pub fn colour_legend_placement(spec: &Spec, plot_path: &str) -> LegendPlacement`
- Modify: `crates/brightfield-shell/src/shelf.rs`: `LEGEND_ROW`, `colour_rows`, `steps`/`stepped`
- Modify: `crates/brightfield-shell/src/app.rs`: `axis_row_written` (the legend row writes `ChartEdit::PlaceColourLegend`)
- Test: `crates/brightfield-spec/src/edit.rs` tests, `crates/brightfield-shell/tests/shelf_settings_colour.rs`, `shelf_settings_colour_window.rs`

**Interfaces:**
- Consumes: `LegendPlacement { Right, Below, None }` and `wire_name()` (`edit.rs:216`); `ChartEdit::PlaceColourLegend { plot, at }` (`edit.rs:205`); `colour_legend_covers` (799); `colour_legends_below` (847, private, which the new function may call).
- Produces: `pub const LEGEND_ROW: &str = "legend"`, `pub const LEGEND_STEPS: [&str; 3] = ["right", "below", "none"]`, `pub fn colour_legend_placement(..)`, and `LegendPlacement::from_wire(&str) -> Option<LegendPlacement>`.

- [ ] **Step 1: Write the failing spec test**

```rust
#[test]
fn a_plots_legend_placement_is_read_off_the_file() {
    for (text, path, want) in [
        (PLOT_WITH_LEGEND_ITEM, "root", LegendPlacement::Right),
        (VCONCAT_WITH_LEGEND_FOR, "root/vconcat[0]", LegendPlacement::Below),
        (PLOT_WITHOUT_LEGEND, "root", LegendPlacement::None),
    ] {
        let spec = parse_spec(text, Format::Yaml).unwrap().spec;
        assert_eq!(colour_legend_placement(&spec, path), want, "{text}");
    }
}
```

Use the fixtures the existing legend-move tests in `edit.rs` use, or copy `PLOT_AT_ROOT` and `PLOT_AT_ROOT_BELOW` from `crates/brightfield-protocol/tests/chart_text.rs:1125-1160`.

- [ ] **Step 2: Implement the reader**

```rust
/// Where the plot at `plot_path` has its colour legend, as the file places it:
/// an item of the plot is at the right, a `legend: color` whose `for:` names
/// the plot is below it, and no node is none.
#[must_use]
pub fn colour_legend_placement(spec: &Spec, plot_path: &str) -> LegendPlacement {
    if !colour_legends_below(spec, plot_path).is_empty() {
        LegendPlacement::Below
    } else if colour_legend_covers(spec, plot_path) {
        LegendPlacement::Right
    } else {
        LegendPlacement::None
    }
}
```

Read `colour_legends_below` and `colour_legend_covers` first. If `covers` already counts a legend below, swap the order so below is asked first, as written above. Adjust the call to `colour_legends_below`'s real signature. Add `from_wire`, the inverse of `wire_name`.

- [ ] **Step 3: The row**

```rust
/// The name of colour's legend row, a head row.
pub const LEGEND_ROW: &str = "legend";
/// Where the legend row steps the legend, in the order `l` steps them.
pub const LEGEND_STEPS: [&str; 3] = ["right", "below", "none"];
const LEGEND_SAYS: &str = "Where the chart's colour legend is drawn: at its right, \
    below it, or not at all. Auto is right.";
```

In `colour_rows`, between scale and reverse: value `colour_legend_placement(spec, &plot_path).wire_name()`, `set: value != "right"`, `kind: Enumerated`, `by_name: false`, no reason. `colour_rows` needs the plot's path. Pass it from `build` (`ChannelSettings` callers have `hero.path`, and the test helper uses `"root"`); add a `path: &str` parameter to `of_plot_profiled`, `of_plot_marked`, `of_plot_drawn` and `of_plot`, and update their callers. `steps()` adds `LEGEND_ROW`. `stepped()` steps through `LEGEND_STEPS` and returns `SettingValue::Word`.

- [ ] **Step 4: The write**

In `app.rs` `axis_row_written`, before the `row_key` lookup:

```rust
        if edit.channel == ShelfChannel::Colour && edit.row == crate::shelf::LEGEND_ROW {
            let at = match &edit.value {
                SettingValue::Word(w) => edit::LegendPlacement::from_wire(w),
                SettingValue::Auto => Some(edit::LegendPlacement::Right),
                _ => None,
            };
            let Some(at) = at else { return Ok(None) };
            if edit::colour_legend_placement(before, &path.0) == at {
                return Ok(None);
            }
            let mut spec = before.clone();
            let placed = ChartEdit::PlaceColourLegend { plot: path.clone(), at };
            edit::apply_for_fresh_load(&mut spec, &placed)
                .map_err(crate::shelf_edit::ShelfRefusal::from)?;
            return Ok(Some((spec, vec![placed])));
        }
```

Check how `shelf_edit` turns a `RefuseReason` into a `ShelfRefusal` (`rg -n "impl From<RefuseReason>|ShelfRefusal::" crates/brightfield-shell/src/shelf_edit.rs`) and use that. A move to below changes the plot's path, and `set_axis_row` rebuilds from the spec and reads the composed plots afresh. Confirm `HERO_PLOT` (index 0) is the plot inside the new `vconcat` after the rebuild.

- [ ] **Step 5: Window test: the legend moves and the rows follow it**

```rust
#[test]
fn the_legend_row_moves_the_legend_below_and_the_list_still_reads_the_plot() {
    let mut w = Window::housing("legend_below");
    w.open_cell(Key::C, "c");
    w.keep_on_colour("median_income");
    w.open_colour_settings();
    w.move_to_row(LEGEND_ROW);
    w.press(Key::L);
    assert_eq!(w.row(LEGEND_ROW).value, "below");
    assert!(w.spec_text().contains("for: "), "below is a legend for the plot");
    assert_eq!(w.row(SCALE_ROW).value, "linear", "the rows still read the plot");
    assert!(w.app.composed().plots[0].legend_below.is_some());
    w.press(Key::L);
    assert_eq!(w.row(LEGEND_ROW).value, "none");
    assert!(!w.spec_text().contains("legend: color"));
}
```

Replace `composed()` with the window's accessor for the composed page (`rg -n "pub fn composed" crates/brightfield-shell/src`).

Make the head-rows tests from Task 3 expect `["scheme", "scale", "legend", "reverse"]`.

- [ ] **Step 6: Run, gate, commit**

```bash
cargo +1.95.0 test -p brightfield-spec && cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour --test shelf_settings_colour_window
cargo xtask ci
git add crates
git commit -m "Give colour a legend row that places the legend right, below or nowhere"
```

---

### Task 8: The ring round the legend while colour's list is open

**Files:**
- Modify: `crates/brightfield-shell/src/legend.rs`: `draw_block` (667) and `draw_below_block` (424) return their drawn `egui::Rect`; `draw_band` (361) and `draw_below` (389) return `Vec<(usize, egui::Rect)>`; new `pub fn ring_rect(block: egui::Rect, data: egui::Rect) -> egui::Rect`
- Modify: `crates/brightfield-shell/src/app.rs`: `ChartDoc` gains `pub colour_list_open: bool` beside `axis_targets_live` (936)
- Modify: `crates/brightfield-shell/src/window.rs`: clear the flag where `axis_targets_live` is cleared (~4050), and set it in `carve_shelf_band` (~10081) when `column_list().map(|l| l.channel()) == Some(ShelfChannel::Colour)`
- Modify: `crates/brightfield-shell/src/chart_item.rs`: draw the ring after the legends (~1440-1465)
- Test: `crates/brightfield-shell/src/legend.rs` tests, a new `crates/brightfield-shell/tests/legend_ring.rs`
- Baselines: two new window baselines

**Interfaces:**
- Consumes: `brightfield_workbench::channel::hue(ShelfChannel::Colour, mode)` and `chrome::colour(..)`; `PlotHandle::data_area()` (`pipeline.rs:319`, page-local).
- Produces: `legend::ring_rect`, `pub const RING_GAP: f32 = 4.0`, `pub const RING_WIDTH: f32 = 1.0`, and `ChartDoc.colour_list_open`.

- [ ] **Step 1: Write the failing unit tests**

In `legend.rs`'s tests:

```rust
/// The ring is the block grown by 4 on every side, and cut back on a side
/// where that would reach into the plot's data area.
#[test]
fn the_ring_stays_outside_the_data_area() {
    let data = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
    let right = egui::Rect::from_min_max(egui::pos2(102.0, 0.0), egui::pos2(200.0, 60.0));
    let ring = ring_rect(right, data);
    assert!(ring.left() >= data.right());
    assert_eq!((ring.right(), ring.bottom()), (204.0, 64.0));

    let below = egui::Rect::from_min_max(egui::pos2(0.0, 101.0), egui::pos2(100.0, 145.0));
    let ring = ring_rect(below, data);
    assert!(ring.top() >= data.bottom());
    assert_eq!(ring.left(), -4.0);
}

/// A block at the right reports the rect it drew: its origin, the block's
/// width, and a bottom at or below its last swatch.
#[test]
fn a_right_block_reports_the_rect_it_drew() {
    let spec = LegendSpec::from_scales(&colour_scale()).expect("a colour scale has a legend");
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))),
        ..Default::default()
    };
    let mut drawn = None;
    let out = ctx.run_ui(raw, |ui| {
        let painter = ui.painter().clone();
        drawn = Some(draw_block(&painter, egui::pos2(10.0, 10.0), 200.0, &spec, None, Mode::Light));
    });
    let drawn = drawn.expect("the frame ran");
    assert_eq!(drawn.min, egui::pos2(10.0, 10.0));
    assert_eq!(drawn.width(), block_width());
    let lowest_swatch = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) => Some(r.rect.bottom()),
            _ => None,
        })
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(drawn.bottom() >= lowest_swatch);
}
```

`colour_scale()` is the tests' helper at line ~1091; check whether `LegendSpec::from_scales` returns an `Option` and whether `ctx.run_ui`'s closure signature matches the neighbouring test's, and follow that test where they differ.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo +1.95.0 test -p brightfield-shell --lib legend::tests`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the rects and `ring_rect`**

`draw_block` returns `egui::Rect::from_min_max(origin, egui::pos2(origin.x + block_width(), used_bottom))`. `used_bottom` is the lowest y it painted at: the last swatch row's bottom for a categorical legend, the ramp's bottom (or its lowest value label, whichever is lower) for a number legend, and the last step's for steps. `draw_below_block` returns the band rect it was handed. Its content is centred in the band, and the band is the block, as the brief's 44-high band reads. `draw_band` and `draw_below` collect `(i, rect)` for each block they draw.

```rust
/// How far the ring stands off the legend block, in points.
pub const RING_GAP: f32 = 4.0;
/// The ring's stroke, in points.
pub const RING_WIDTH: f32 = 1.0;

/// The ring round a legend `block`: the block grown by [`RING_GAP`], cut back
/// on a side where that would reach into `data`, the plot's data area, so the
/// ring never lies over a mark.
#[must_use]
pub fn ring_rect(block: egui::Rect, data: egui::Rect) -> egui::Rect {
    let mut ring = block.expand(RING_GAP);
    if block.left() >= data.right() {
        ring.min.x = ring.min.x.max(data.right());
    }
    if block.top() >= data.bottom() {
        ring.min.y = ring.min.y.max(data.bottom());
    }
    ring
}
```

- [ ] **Step 4: The flag and the draw**

`ChartDoc` gains `colour_list_open: bool`, documented like `axis_targets_live`: cleared each frame and set while colour's list is open on either tab. Set it in `window.rs` where `axis_targets_live` is set.

In `chart_item.rs`, keep the rects the legend calls return. After both legend calls, while `doc.colour_list_open` is true, ring the hero's block:

```rust
        if doc.colour_list_open {
            let hero = rights
                .iter()
                .chain(belows.iter())
                .find(|(i, _)| *i == crate::window::HERO_PLOT)
                .map(|(_, r)| *r);
            if let (Some(block), Some(plot)) = (hero, doc.composed.plots.get(crate::window::HERO_PLOT)) {
                let area = plot.data_area();
                let data = egui::Rect::from_min_size(
                    rect.min + egui::vec2(area.x as f32, area.y as f32),
                    egui::vec2(area.width as f32, area.height as f32),
                );
                let hue = chrome::colour(channel::hue(ShelfChannel::Colour, mode));
                ui.painter().rect_stroke(
                    legend::ring_rect(block, data),
                    0.0,
                    egui::Stroke::new(legend::RING_WIDTH, hue),
                    egui::StrokeKind::Inside,
                );
            }
        }
```

`rect` is the raster rect in this scope (`doc.raster_rect`). Check `HERO_PLOT`'s visibility (`pub(crate)`) and the import paths of `chrome` and `channel` used elsewhere in `chart_item.rs`. With no legend there is no block, and nothing is drawn.

- [ ] **Step 5: Window tests**

New `tests/legend_ring.rs`. Build the window as `shelf_settings_colour_window.rs`'s `Window::housing` does, either by copying its struct or, if the tests share a module, by importing it. Assert on the stroke shapes of the frame (as `shelf_settings_colour.rs` reads its ring stroke):
- at rest, with colour holding `median_income`: no stroke in colour's hue;
- with colour's list open on columns, then on settings: one stroke in colour's hue, its rect outside the hero's data area (`hero_data_area()` as in `tests/grid_spot.rs:153`);
- after the legend row steps to below: the stroke encloses the band under the plot;
- after it steps to none: no stroke.

Two baselines, `legend_ring_right_{light}` and `legend_ring_below_{light}`, through `capture_png` as `tests/legend_below_baseline.rs:146` does, with colour's list open. Generate, read, and check the ring is 1 px and clear of the plot.

- [ ] **Step 6: Run, gate, commit**

```bash
cargo +1.95.0 test -p brightfield-shell --lib legend::tests && cargo +1.95.0 test -p brightfield-shell --test legend_ring --test legend_below --test legend_baseline
cargo xtask ci
git add crates/brightfield-shell
git commit -m "Ring the legend in colour's hue while colour's list is open"
```

---

### Task 9: The reverse row's hand-off, held by a window test

**Files:**
- Test: `crates/brightfield-shell/tests/shelf_settings_colour_window.rs`
- Modify (comments only): `crates/brightfield-shell/src/shelf.rs:697`, `crates/brightfield-shell/src/window.rs:5623-5624`, `crates/brightfield-shell/tests/shelf_fallback.rs:1128` and `:1143`

**Interfaces:**
- Consumes: `hero_axis_settings` as Task 3 left it (it calls `of_plot_profiled` with `&hero.marks`).

- [ ] **Step 1: Write the test**

The hero must draw no dot and have a column on colour. A `cell` mark with `fill: <number column>` over the housing CSV does. The `Window` struct opens a data file. Add a `Window::spec(name, text)` constructor that writes `text` beside a copy of the CSV fixture and opens it with `Boot::open`, as `reopen()` does, or loads it as `tests/legend_below.rs:82` `laid_out` does with `Boot { live: Some(live), .. }`. Then:

```rust
/// The window hands colour's settings the marks the hero drew, which is what
/// mutes reverse on a chart with no dot. Every other window test's hero draws
/// dots, so this is the one that tells the marks drawn from no marks at all.
#[test]
fn reverse_is_muted_on_a_hero_that_draws_no_dot() {
    let mut w = Window::spec("cells", CELL_HERO);
    w.open_colour_settings();
    let reverse = w.row(REVERSE_ROW);
    assert!(
        reverse.reason.as_deref().is_some_and(|r| r.contains("dot")),
        "reverse says why it does not apply, got {:?}",
        reverse.reason
    );
}
```

`CELL_HERO`: a one-plot spec over the copied CSV with `mark: cell`, `x: { bin: median_income }`, `y: { bin: house_age }`, `fill: median_house_value`. Check `cell`'s channel syntax against `tests/colour_vocabulary_reverse.rs:520` `mark_specs`. Before relying on it, confirm the shelf band opens on this hero (`open_cell(Key::C, "c")` reaches the colour cell). If the band does not open on a cell chart, use a `raster` or `heatmap` hero from the same `mark_specs`, and record which in the test's doc.

- [ ] **Step 2: Prove it guards the hand-off**

Run it: `cargo +1.95.0 test -p brightfield-shell --test shelf_settings_colour_window reverse_is_muted_on_a_hero_that_draws_no_dot`. Expected: PASS.

Then temporarily replace `ChannelSettings::of_plot_profiled(spec, plot, channels, &hero.scales, &hero.marks, moments)` in `hero_axis_settings` with `ChannelSettings::of_plot_drawn(spec, plot, channels, &hero.scales)` and run it again. Expected: FAIL. Restore the line with `git checkout -- crates/brightfield-shell/src/window.rs` if nothing else in that file is uncommitted; otherwise undo the one edit by hand. Run it once more. Expected: PASS.

- [ ] **Step 3: Correct the stale comments**

- `shelf.rs:697`: "The settings list: the rows of an axis or of colour, read from the plot."
- `window.rs:5623-5624`: "`Tab` turned the list to the channel's settings, an axis's or colour's: the preview is backed out of, and the rows read the chart as it was kept."
- `tests/shelf_fallback.rs:1128` and `:1143`: say "the axes' and colour's settings" where they say "the axes'". Read each line in context first and change only the words that are now wrong.

- [ ] **Step 4: Gate and commit**

```bash
cargo xtask ci
git add crates/brightfield-shell
git commit -m "Hold the hand-off of drawn marks to colour's settings with a window test"
```

---

### Task 10: Whole-suite run and the pull request

- [ ] **Step 1: Run the whole suite**

Run: `cargo +1.95.0 test --locked --workspace --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/colour-suite.txt | rg "^test result|FAILED|failed"`
Expected: every result line `ok`, apart from the two `remote_start` tests that fail on a Mac on `main`. Sum the passed, failed and ignored counts for the PR.

- [ ] **Step 2: Push and open the PR**

Push the branch. Open the PR with a body that:
- says what the analyst can now do (the four rows and the ring) in plain words;
- lists every baseline that moved, file by file, with what moved (Task 1's legends; the new snapshots);
- quotes the suite totals and names the two `remote_start` failures;
- names the deliberate difference: steps drawn in the count asked for.

End the body with the attribution line. Do not merge: say it is ready, with its link.
