# An operator step's values in the inspector

## Intent

An analyst who selects a node an operator made, such as the Parquet a UMAP projection writes, sees in the inspector's Step section every value that step's operator takes. Each value is drawn so the analyst can learn how the operator works: a number on its range, a choice among its options, the values the step sets told apart from those it leaves at their defaults. The owner's words: configuration is "an opportunity for education - like adjusting hyperparameters", and the chosen look is "more visual, more intuitive and inviting... the type of feature that meets the definition of fun more closely and give users a space to learn how the operator works."

The panel is read-only. It is the surface a later change makes editable.

## Goals

1. Selecting a node an operator step writes, on the canvas or in the spine, shows that step's values in the inspector's Step section, after the node's existing facts.
2. Every value in the operator's description is shown, and no value it does not describe. A value is drawn by its shape:
   - **A number with both limits** in its schema (`minimum`, and `maximum` or `exclusiveMaximum`, or a bound worked out from the data) is a bar from one limit to the other with a mark at the value in force. The limits are written at the bar's two ends as numbers.
   - **A choice** (`enum`) shows every option, with the one in force filled.
   - **Anything else** (a number with an open end, text, a path, a list, a nested object) is a plain row: the key and the value, a nested value indented beneath it.
3. A value the step sets is told apart from one left at its default, in ink and with a tag. A value with no default that the step leaves unset reads "not set".
4. Under each value, the first sentence of arc's description of it. The rest of the description folds behind "More".
5. A bound that depends on the data is worked out on the input the step actually reads and drawn as the bar's far end. For UMAP's `neighbors` on an input of 48 rows, the bar runs from 2 to 47, and an unset `neighbors` shows 15 (or the bound, if that is smaller). Before the input exists, the far end reads that it is known once the step that writes the input runs, naming that step.
6. After the values: what the step reads and writes, each by role, with the path or columns the step names, and "not set" for an optional role left out. A role both read and written (UMAP's `fit`) appears in both.
7. Arc works out all of it. Brightfield asks arc for the step and draws the answer (see Approaches).

## Non-goals

- Changing a value in the panel. That is separate work, and the bar is drawn so it does not look draggable until then.
- Drawing an operator's results: match weights, feature importance, a projection's fit. Separate work.
- Keeping a record of each run's settings and results. Separate work, informed by a study of MLflow.
- Moving a Splink model's settings from its settings file into the step's `with:`, and having the step train. Separate arc work; see Splink below.
- Selecting a step itself, apart from the node it makes. Not needed: the owner confirmed that selecting the node an operator made is how the panel appears.
- A step that is not an operator's (SQL, a command, an operation from the catalogue). Its Step section is unchanged.

## Anti-goals

- **A sea of dials.** The owner's warning is about cloud consoles' settings pages. The panel must read as a picture of the operator's knobs, not a form.
- **A bar that misleads.** A bar drawn to a straight-line scale over a huge range puts every sensible value against its left end and implies the whole range is reasonable. See the scale rule below.
- **Limiting what an operator writes.** The owner's general rule: no limit on the outputs of any operator. Every role in `writes` is shown.

## Constraints

- **Arc owns the spec and runs it; Brightfield edits it.** The values, defaults and bounds come from arc, the same answer any other surface (a hosted run's page, an agent through arc's MCP server) gets.
- **Local and online parity.** A Protocol behaves the same wherever it runs, and the bound shown is the one arc will enforce on the input the run will read, counted by arc.
- **Brightfield reaches arc by running itself as its pinned arc** (the window's runner), not through a library call. A new arc command reaches Brightfield through a pin bump.
- **Arc refuses `with:` keys its operator does not describe** (`deny_unknown_fields`), so drawing exactly the description's keys shows every value the step can hold.
- **Public repos.** Brightfield and arcform are public; text in them names no private planning identifiers.

## Approaches

Two were weighed: who works out the values the panel draws.

- **A. Brightfield works them out from `arc operation describe`.** It keeps the schema's types, defaults, limits and `x-at-most`, keeps each step's `with:` on its record, counts the input's rows with its own DuckDB, and a test checks that arc refuses one past the bound Brightfield worked out. Nothing waits on arc. But the rule is read in two programs, every other surface has to read it again, and Brightfield may count a different copy of the input from the one a run on another host reads.
- **B. Arc answers for the step.** A new arc command takes a Protocol and a step and returns the operator's description with the values set, the defaults in force and the bounds worked out. Brightfield only draws.

**The owner chose B**, for local and online parity: one program, arc, says what a step's values are, wherever it is asked from. Its cost is that the Brightfield work waits on the arc work and a pin bump.

## The design

### In arc (arcform)

**A command that describes a step.** `arc step describe [--dir <protocol>] <step>`, and an MCP tool of the same meaning beside `operator_describe`, answer in JSON:

- the operator's name, version, summary, and the `reads`, `writes` and `parameters` exactly as `arc operation describe` gives them;
- for each role: the path or list the step's `with:` gives, or none;
- for each parameter: the value the step's `with:` sets, or none; the value in force (the set value, else the default with any data bound applied to it, as `bounds_default` says); and, for a parameter with `x-at-most`, the bound worked out: the number, the role and path whose rows were counted, and the row count; or, when that input cannot be counted yet, why, naming the step that writes it when the Protocol has one.

**A value past its bound is answered, not refused.** A step may set `neighbors: 100` on an input of 48 rows, or its input may shrink after the value was set; the manifest still loads, since the bound is checked at run. The command answers with the value, the bound, and a sentence saying the run will refuse it, in the words the run's refusal uses. *(The assistant's call.)*

**Which file is counted.** The command counts the input as it stands on disk. When the step that writes that input is stale or has not run, the answer says so, naming that step, so a client can say the bound may change once it runs; when the file does not exist, there is no bound yet and the answer names the step that writes it, or says no step of the Protocol does. *(The assistant's call: counting what is on disk is what a run started now would count first, and the stale writer is said rather than guessed past.)*

A step that is not an operator's is refused with a sentence saying so. Whether the command also answers for a step recorded from arc's operation catalogue is the arc builder's call; Brightfield does not need it.

**One reading of the bound.** Today `umap_project`'s run-time refusal of `neighbors` is written by hand for that operator, beside the schema's `x-at-most`. The command and the run's check read the same rule from the same code, so what the panel shows and what a run refuses cannot disagree. The rows are counted from the Parquet footer, the way the run counts them, with no rows read.

**Words for a bar's two ends.** A numeric parameter's schema may carry `x-ends`: two short words saying what the low and the high end of its range mean, for example `["local", "global"]` for `neighbors` and `["packed", "spread"]` for `min_dist`. They are arc's words, as a value's sentence is. `umap_project` carries them for its two numbers. Other operators gain them as their authors write them.

**Nested values pass through.** A parameter whose `with:` value is a list or an object is returned as it stands, with its schema, so a client can draw it. Nothing in the command is shaped for Splink's settings file.

### In Brightfield

**The pin.** Brightfield's pinned arc moves to the arcform commit that carries the command. Its other changes in that range are taken as they come, under the gate.

**Asking arc.** When the node in the Step section was made by an operator step, the window asks arc for that step on a worker, off the UI thread, as `operation.rs`'s description reader asks for an operation. The answer is kept per step and asked again when the Protocol's manifest changes or a run finishes, since either can change what the step reads or the rows it counts. While the answer is on its way, the Step section shows the node's facts and a quiet line in place of the values. A refusal from arc is shown as arc's sentence.

**Drawing.** The Step section's body (`protocol.rs`'s inspector body) draws, after the node's facts:

1. **Values**, in arc's order where the answer keeps it, else by key.
   - A bar where both limits are known. **The scale is logarithmic when the lower limit is above 0 and the upper limit is more than 100 times it**, otherwise straight; `exclusiveMaximum` counts as the upper limit. So `neighbors` from 2 to 47 is straight, from 2 to 26,528 is logarithmic, and `min_dist` from 0 to 1 is straight. *(The assistant's call.)* The two ends are labelled with `x-ends` when arc gives them, above the limits as numbers.
   - The options of a choice as a row of chips, the one in force filled.
   - A plain row otherwise; a list or object indented beneath its key.
   - A value past its bound has its mark pinned at the bar's far end in the warning ink, with arc's sentence that the run will refuse it beneath.
   - Where the input's writer is stale, the far end's number carries "may change once `<step>` runs".
   - The key in full ink with a "set" tag when the step sets it; in muted ink with a "default" tag when it is the default; "not set" when it has neither.
   - The first sentence of arc's description beneath, the rest behind "More".
2. **Reads** and **Writes**, each role with what the step names, "not set" for an optional role left out.

The bar, the chips and the plain row are drawn from the schema alone and serve every operator. They are drawn as readouts: no handle, no hover state that suggests a drag.

### Splink

`splink@1` reads a list of inputs, each by role, and its settings sit in a Splink settings file it names under `model`. Under this design its panel shows the inputs, `model` as a path, and its two thresholds (`threshold_match_probability` as a bar from 0 to 1). The owner ruled that a Splink model's settings belong in the Protocol YAML. That splits Splink's settings file in two:

- **settings**, which the analyst chooses: the link type, the blocking rules, each comparison and its levels' conditions, the prior, EM's iterations and convergence; these go in `with:`;
- **results**, which training works out: each level's m and u probabilities and the probability that two random records match; these are a file the step writes and keeps, the way UMAP keeps `fit`.

That is its own arc work. When it lands, the panel draws the settings with no change here, as nested values.

## Testing

- **A committed fixture Protocol** under Brightfield's examples: one `umap_project` step reading a small committed Parquet of 48 rows with a numeric column, setting `metric: cosine` and leaving `neighbors` and `min_dist` unset, with `fit` unset. No sibling checkout and no network.
- **The panel's contract test** opens the fixture, selects the node the step writes and asserts the Step section draws exactly the description's parameters; `metric` as set, `neighbors` and `min_dist` as defaults; `neighbors`' bar running from 2 to 47 with 15 in force; `fit` as not set under both Reads and Writes.
- **Before the input exists**: a second fixture in which a SQL step, not run, writes the Parquet the UMAP step reads, asserting the far end says it is known once that step runs. The test reads only arc's answer; nothing is run.
- **Past the bound**: the first fixture with `neighbors: 100` set, asserting the mark is pinned at the far end and arc's sentence is shown.
- **In arc**: a value past the bound is answered with the run's refusal sentence; an input whose writer is stale names the writer.
- **A screenshot** of the Step section on the fixture, light and dark, drawn on the Mac Studio.
- **In arc**: the command's answer on a 48-row input is 47; a run with `neighbors: 48` is refused by the same code; an unset `neighbors` on a 10-row input is in force at 9; `x-ends` passes through.

## Sequencing

The arc work is one pull request in arcform and the Brightfield work one in Brightfield. Brightfield's pin may point at the arc branch's head while both are open, so the Brightfield work is built against the real command; before Brightfield's pull request is called ready, the pin moves to the arc commit on arcform's `main` once the owner has merged it. The arc command's JSON is fixed in arc's pull request before Brightfield draws from it. *(The assistant's call.)*

## Left to the builder

- The arc command's exact JSON field names, and whether it answers for catalogue operations.
- How the window decides an answer is stale beyond "the manifest changed or a run finished".
- Where the bar, chip and plain-row drawing lives (a module of its own is the likely home) and their exact sizes, inside the design system's tokens.
- How "More" opens.
