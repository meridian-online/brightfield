# Getting-started dashboards, and the skill that composes them

*Design, 2026-10-25. Written from a brainstorm with Hugh; his words are in quotation marks. Everything not quoted is a proposal or a call made on his behalf, and the calls are listed at the end.*

## Intent

An analyst opens Brightfield, picks a well-known dataset from the start gallery, and lands on a dashboard that shows them something about that data they had not seen. Then they point the same thing at their own files and get the same experience.

The dashboard is the point, and it is composed, not templated. The model is Embedding Atlas: it "takes something well known and really expands on it — offering new visualisations that are linked, thereby an interaction with the data which is a differentiated experience. It builds intuition that can only come from a well composed toolset."

What composes it is an **analysis skill**: instructions an analyst's coding agent follows to explore a dataset and compose a linked, crossfiltered dashboard whose views follow from what the data says. The skill is the milestone's worthwhile outcome. The five gallery dashboards are its first outputs, so the gallery shows on well-known data what the skill does on yours.

### Why now

The launch waits on the first hour with Brightfield giving five feelings Hugh named: "this is easy"; "aha! I can see it now"; "wow, this gives me more than a spreadsheet"; "results are so quick"; "I need to tell … about this". The gallery's real data is one dataset (California Housing) and one Protocol shown three ways (the EDGAR ↔ GLEIF crosswalk); the three other starts draw generated readings and are off the first screen. This work carries the first two ingredients of that hour, well-known datasets and Protocols that draw them, and the skill carries "aha! I can see it now" onto the analyst's own data.

The practice of analysis is, in Hugh's words, "crucial to best reflect what the Meridian framework can achieve". A published skill is also a discovery channel: skills registries list a skill by its own name, and the data-analysis entries there have a few thousand installs at most.

## Who it is for

Analytics authors with middling-to-high technical depth who use a coding agent (Claude Code, Codex, Cursor, Gemini CLI) and are early adopters. They install the skill through a plugin or a skills registry, point their agent at a file or a folder, and look at the result in Brightfield.

## What good looks like

**"Sparking joy."** "We want to have wonderful results, while making it feel easy and 'in flow'." The joyful moment is the first grid and dashboard arriving. So the experience follows the conditions for flow: a clear goal, immediate feedback, nothing that breaks concentration.

- **Something good arrives fast.** The analyst sees a result before the agent has finished thinking about the story, not after a round of questions.
- **The composed dashboard follows, and its story is stated as a claim** ("price is driven by size and region, not age"), which the views let the analyst test by brushing.
- **At most one question,** asked only when it helps: when the agent has two credible stories, or several sources whose relationships it cannot confirm. Otherwise it acts, and says what it assumed in a line the analyst can override.

## Goals

1. **The skill.** An agent following it takes one data file or a folder of related files to a composed, linked dashboard in a Protocol, with no template: the views follow from what the data says.
2. **It is published where analysts find skills,** in a public repository named `meridian-online/data-analysis-skills`, beside arcform's five analyst skills.
3. **The gallery opens onto five well-known datasets,** each a Protocol the skill produced, drawing its composed dashboard: Palmer Penguins (academia), NYC TLC yellow taxi trips (commerce), UCI Online Retail II (retail), Epoch AI's notable AI models (topical), and USGS earthquakes of magnitude 4.5 and up in 2025 (geographic).
4. **Every dashboard rests on a grid** that is near and quick to reach, linked to the views in both directions.
5. **Data quality is near every insight,** so "any leap from insight to action is tempered with a reasonable view of accuracy of ground data".

## Non-goals

- **Starting the skill from inside Brightfield.** Hugh wants an analyst to be able to start it from the window, at the new-Protocol moment (opening a folder of files or an online response) or the front door. How Brightfield and agents relate is unresolved and has its own brainstorm. Here the skill runs in the analyst's coding agent, arcform's existing route, and Brightfield redraws as the Protocol's panels arrive.
- **Linking separate panel tiles to each other.** A Protocol's panels draw today as separate tiles with nothing shared between them. Linked views live inside one panel here; cross-tile linking belongs with the agent brainstorm, since it changes what the window is.
- **The ERD for multi-source data.** Drawing how an analyst's tables relate, through dovetail, is the separate milestone for an analyst's own files. The skill uses relationships it can establish through arc and says when it cannot.
- **New chart marks.** The skill composes from the marks Brightfield draws. A mark it needs and cannot get is filed as a finding.
- **Windows and Linux,** at launch.

## Anti-goals

Each is a failure even if the work technically ships.

- **A template.** "We can't be too literal. Any specific visualisation shouldn't be mandated. This should really be specific to the story the data is telling." A dashboard that is a KPI row, a trend and a table for every dataset has failed, and so has one that is a histogram per column with a scatter on top.
- **A chart gallery dressed as a story:** equal-weight tiles with no focal view and no stated claim.
- **An insight on compromised data with nothing nearby to say so.**
- **An interrogation:** an agent that opens with questions it could have answered by looking.
- **A dataset whose licence was read from a mirror.** Each dataset's terms are read at its issuing body, not from a re-uploader's label.

## Constraints

- **Licence and route, per dataset**, from Meridian's licensing review of 2026-10-24. Penguins (CC0) and the USGS earthquakes (US public domain) are bundled: their issuer permits redistribution, they are small, and each earns a start that draws offline. Taxi, Online Retail II and Epoch AI are fetched from the issuing body's own URL, and their starts say "over the network" on their button. The CC BY datasets (Online Retail II, Epoch AI) carry their credit in the Protocol's descriptor and onto any exported picture. Epoch's `Authors` and `Abstract` columns and Online Retail's `CustomerID` are left out of anything shown.
- **The bundle profile.** A third-party dataset is bundled only when its issuer permits redistribution and transformation, it is small (the reference sizes are Penguins' CSV at 15,241 bytes and a year of earthquakes as Parquet at 211,366 bytes), and it earns an offline start. The earthquakes bundle the 8,502 events of type `earthquake` from the USGS's own network (`net = us`), which keeps the public-domain claim exact.
- **The skills are versioned to the binary.** arcform bundles its skills into `arc` (`arc skills install`), marks each with the arc version it teaches, and tests every verb a skill names against the real binary. The new skill joins that arrangement rather than leaving it.
- **The door's size.** Four starts are on the door today. The gallery decision of 2026-08-07 says each start earns its slot by a different chart vocabulary, and to re-decide past seven. Five more would make nine; Hugh re-decided it, below.
- **Public repos.** brightfield, arcform and the new skills repository are public: no private planning identifiers in their files, commits or pull requests.

## Approaches

Three ways were weighed.

- **A′. Write and test in arcform, publish to `meridian-online/data-analysis-skills`.** The skill is authored beside arcform's five, tested against `arc`, bundled into the binary and judged in arcform's analyst eval. arc's release workflow publishes all six skills to the public skills repository on each tag. The gallery starts are Protocols the skill produced. *Trades:* a second repository and a sync step in the release, and the skills repository takes changes only through arcform. *Gains:* a discoverable home without losing the tests or the bundling.
- **B. The skills repository as the only source.** The skills move out of arcform, arc fetches or vendors them, and the binary-backed tests are rebuilt there. *Trades:* reworking arcform's skill packaging while its first skills release is being evaluated. *Gains:* the discovery repository is the real home, and outside contributions land directly.
- **C. Hand-compose the five gallery dashboards first, then write the skill from what they teach.** *Trades:* the gallery stops being evidence the skill works, and the skill risks being written to justify outputs that already exist, the template problem.

**Picked: A′** (Hugh, 2026-10-25, naming the repository `meridian-online/data-analysis-skills`; the reading of that as A′ rather than B is a call, below). Hugh's reason for a separate public repository is discovery: "it could act as a discovery tool, e.g. ranking high in all the places that the other skills you found are", the way Superpowers ranks in skills registries and plugin libraries. Its name has to say what the skills do and why someone would install them; a repository called `skills` "would wash out".

## The design

### 1. The skill

`dashboard-from-data`, a new skill in arcform's `skills/`, beside `scoping-an-analysis`, `profiling-and-typing-data`, `building-and-checking-a-protocol`, `presenting-findings` and `using-arcform`. It is for the question "show me what's in this", where arcform's existing route is for "answer this question": `presenting-findings` writes one question per chart, which is the opposite of composing linked exploration.

**What it does, in order:**

1. **Size from the data and the ask, not by interview.** One tidy file, a wide file, or a folder of related files; a directed question or an open one. It says what it sees in one line and carries on; the analyst can override it. The prior art supports sizing on these terms: every analysis profiles, discovery happens in open-ended analyses, many sources need reconnaissance before tasks are known, and on wide data choosing variables is the hard part.
2. **Profile, through `profiling-and-typing-data`.** FineType types every column, the schema is saved and every row validated. The quality findings are kept for step 6, not reported as a gate the analyst has to pass first.
3. **Show something good fast.** The first thing the analyst sees is a result: the grid and the dashboard Brightfield already draws for an opened data file, or the first panel the skill writes. The point is the time to the first picture, not which picture.
4. **Explore to find the story.** An iterative loop of queries through arc, each chosen from the last result, broad first and then into segments, inside a stated budget. It looks for what drives the data: distributions and their shapes (two peaks as two populations), which dimension splits a measure most, correlations and where they reverse within a segment, change over time, and where overplotting hides structure. For a well-known dataset the agent may bring what is known about it, and says so. Every number it states comes from a query it ran.
5. **State the story as a claim, and check it against the data.** The planned views are run before they are composed, and a view that comes out flat, empty or uninformative is dropped rather than shown.
6. **Compose one linked panel.** A focal view that carries the claim, and around it the views that let the analyst test it, all on one shared crossfilter selection, so brushing one isolates the mechanism in the others. The skill decides which view drives and which respond. No view type is required and no count is fixed. Two things are always within reach rather than always on the first screen: the grid of the table the dashboard rests on, and a view of the data's quality that follows the selection (proposed: a line such as "20,433 of 20,640 rows complete; `total_bedrooms` missing in 207" that updates for the brushed rows). A claim that rests on a weak column says so in its own words.
7. **Check what was made.** `panel_check` clean, and the story, intuition and honesty tests from the skills read, run against its own queries: the takeaway fits in one sentence, the dashboard reads without hover, and nothing is claimed as causal that is only associated.
8. **Hand over** through `presenting-findings`' route: open the Protocol in Brightfield, with the claim and its caveats in the chat.

**Method sources.** The skill's references distil what the brainstorm read: the profiling-to-column-roles method of Anthropic's data plugin, the claim-first composition and anti-patterns of OpenAI's visualisation-strategy skill, "no fixed checklist" (OpenAI) and "defaults, not quotas" (MotherDuck), Rill's query loop and its rule that every number comes from an executed query, Databricks' story confirmation, and the honesty gates of the storytelling and EDA skills; behind them Tukey, Cleveland and McGill, Munzner, Shneiderman, and for checking back, Horvitz and Heer. None of those skills says which view in a crossfilter should drive which; that is the part this skill adds. The skills are read for method and their text is not copied; OpenAI's data-analytics plugin is proprietary.

### 2. Publishing

`meridian-online/data-analysis-skills` is created as a public repository. arc's release workflow writes the six analyst skill folders into it on each tag, so a skills registry can list them and an analyst can install them without cloning arcform. Plugin libraries keep installing arcform's own plugin. arcform's `skills/` stays the source; the repository says so in its README and points changes there. Its README says the skills need `arc` installed and how to install it: unlike Superpowers, these skills drive a tool, and that install is what turns a skill install into an arc user.

The new skill is named for its outcome, `dashboard-from-data`, the way the most-installed skills are (`frontend-design`, `tdd`); the five existing skills keep their names (see the calls below).

### 3. The gallery

Each of the five datasets becomes a Protocol produced by running the skill on it through the agent route, then reviewed and committed into brightfield as a start. A Protocol is the folder arc writes: the manifest, the load step (a fetch from the issuing body's URL, or the bundled file), the saved schema and the panel that holds the composed dashboard.

**A new kind of start opens a shipped Protocol.** Today a start opens a chart spec, a Protocol manifest with no run, a Protocol with a run record, or a data file it fetches. The new kind writes the shipped Protocol to the datasets directory, runs it with Brightfield's own arc runner when it has no run yet, and draws its panel, so a gallery click lands where an analyst's own run of the skill would. A bundled dataset's Protocol runs offline. A fetched one says "over the network" on its button, and what it does with no network is stated: a banner naming the network and the URL, over a window that stays up, as the crosswalk chart's and California Housing's starts do today.

**Thumbnails** are drawn from the shipped data or a committed reduced copy of it, the way the crosswalk chart's are, by the regeneration test.

### 4. The grid

"Every dashboard is based on a grid, at least one. As long as that grid is nearby and quickly accessible, analysts will be able to have their row level questions answered." Whether it shares the first screen is the composition's call. Wherever it is, it takes part in the crossfilter both ways: a brush filters the grid, and a filter typed in the grid filters the views. Today the second direction does not hold (on 0.1.7, a typed filter on `house_age` changed the grid and not the map), and fixing it is part of this work.

### 5. Data quality

"Data quality shouldn't be the first view, it should be nearby." The skill composes it into the panel from what profiling and validation found, over the same table and the same selection as the views. If Brightfield cannot draw a view that follows the selection this way, the builder says so before working around it.

## How it is checked

- **The skill against arc:** arcform's skill tests, as for the other five.
- **The skill's output:** arcform's analyst eval gains tasks for this skill, on datasets outside the gallery, so the gallery does not grade itself.
- **The gallery in Brightfield:** each new start loads, its fetch-less form runs in the hermetic tests, its network form in the network tests, and its thumbnails regenerate, as for the existing starts.
- **The grid's filter:** a typed filter in a dashboard's grid filters its charts.
- **The feeling:** Hugh opens the five starts and runs the skill on a file of his own, and records whether the first hour gives the feelings he named. That is the acceptance, and nothing ships to the launch until he has.

## Left to the builder

- Each gallery dashboard's composition: that is the skill's job, reviewed by Hugh.
- The query budget and the exact wording of the skill and its references.
- How much of the taxi data a start fetches (one month is 64 MB of Parquet), and how Online Retail II's spreadsheet is read.
- How the new start kind writes, runs and caches its Protocol, within the rules above.
- How arc's release workflow writes to the skills repository.

## Decided by Hugh after review

Hugh, 2026-10-25, answering the questions the design review raised:

- **The door shows six:** the five new starts and the crosswalk as a run. Each draws a different picture: Penguins' measures by species, the earthquakes on a map, the taxi trips' rhythms, Online Retail's customers over time, AI training compute over time, and one Protocol as the run it records. California Housing, the crosswalk declaration and the crosswalk chart leave the door and stay in the binary, so an empty pane's button and a restored session still open them. The earthquakes become the door's map. The module documentation on the gallery's size is amended in the same change.
- **The skills repository sits beside arcform's plugin.** `meridian-online/data-analysis-skills` holds the six skill folders, which is what skills registries read. Plugin libraries keep installing arcform's plugin from arcform as they do now; its manifests are not moved.
- **The agent creates the repository; Hugh adds the credential.** The build creates `meridian-online/data-analysis-skills` as a public repository with a README that says the skills are written in arcform and points changes there. Hugh adds a write token as a secret on arcform's release workflow, which an agent cannot make. That task stops for him.

## Calls made on Hugh's behalf

- Reading "let's go with `meridian-online/data-analysis-skills`" as choosing A′ (write in arcform, publish there) rather than B.
- Linked views inside one panel for this work; cross-tile linking deferred.
- The grid's typed filter reaching the charts is in scope.
- The skill's first picture may be Brightfield's existing dashboard for an opened data file.
- The quality view is composed by the skill into the panel rather than built into the window.
- The skill's eval uses datasets outside the gallery.
- **The names.** The new skill is `dashboard-from-data`, named for its outcome. The five existing skills keep their names everywhere, in arcform and in the published copies: renaming them is a migration for analysts who already have them installed, for no gain this work needs. A later rename is one frontmatter field and a release note.
- **The render check (design step 7)** is not a step the skill can require. The skill can only name what `arc` advertises, and Brightfield's headless export is not something `arc` advertises. The skill's check is `panel_check` clean plus the story, intuition and honesty tests against its own queries. Rendering and looking at each panel is required when the gallery is authored, using Brightfield's own export. A render verb an analyst's agent can call is filed as a follow-up.
- **Where a start lands.** A shipped Protocol with one panel opens straight into that panel as the chart document, with the Data pane holding its grid, not into the tile view. The builder confirms the chart document shows the Data pane for a Protocol's panel, and the grid's typed filter is fixed there as well as in an opened data file's dashboard.
- **The credit for CC BY data** goes in two places that exist today, with no schema change: a Frictionless descriptor shipped in the Protocol folder naming the issuing body, its terms and its citation (the shape California Housing's start already carries), and a credit line the panel draws as its own text, so an exported picture carries it. A credit field in arc's manifest, if it is wanted, is a separate change.
- **The eval.** The new skill's tasks get their own rubric: time to the first picture, a stated claim checked against the data, quality within reach, at most one question, and no template. The existing pass bar's "plays the plan back" outcome does not apply to them. `using-arcform` routes "show me what's in this" to `dashboard-from-data`, which sizes the work itself in one line rather than through `scoping-an-analysis`. Hugh approves the new rubric before the eval is scored against it, as he did the existing pass bar.
- **Fetched data is not pinned.** Epoch AI's file grows as models are released, and that is why it was picked. Each fetched start reads its issuer's live URL. Its dashboard's claim is written to hold as the data grows (the shape of a trend, not a figure that will change), its thumbnail and hermetic tests read a committed reduced copy, and its network test checks the columns and shape the Protocol expects, not values.
