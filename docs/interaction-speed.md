# What makes an interaction fast

Brightfield pushes interaction down into the database: dragging a brush, moving a
slider or zooming a plot becomes a predicate and a re-query, not a filter over
data held in the client. What crosses into memory is the query's answer rather
than the table — and how large that answer is depends on the mark. An
aggregating mark returns its bins; a row-level mark returns a row per drawn
point, which at ten million rows is the whole column set. The `Arrow held`
column of the measured record below has both figures.

On top of that sits **pre-aggregation**. The first time you interact with a plot
that summarises its data, brightfield builds a small summary table — a *cube* —
keyed on the thing you are changing. Every later gesture reads the cube instead
of the source. The cube is usually thousands of rows where the source is
millions, which is where the speed comes from.

**The honest version of the claim is that interaction cost stops tracking row
count *when a cube can be built*.** It is worth knowing when that is, because the
difference is large and it is a property of your chart rather than of your
machine.

## When you get a cube

**The mark has to summarise.** A density plot, a binned density, a histogram, a
heatmap or a bar chart all reduce many rows to few marks — so there is something
to pre-compute. A raw scatter does not: every row is its own dot, and a summary
of "every row, individually" is the data again.

**The thing you are changing has to have bounded distinct values.** The cube is
keyed on the column your gesture moves. If that column has forty distinct values,
the cube has forty slots per bin. If it has one distinct value per row, the cube
has as many rows as the source and buys nothing.

## Measured

Ten million rows, on an Apple M1 Pro, median with the 95th percentile beside it.
Every cell below is read from
[`benchmarks/results/2026-08-07-apple-m1-pro.json`](../benchmarks/results/2026-08-07-apple-m1-pro.json),
which carries the methodology beside the numbers.
`scripts/check-measured-figures.py` re-reads that record on every pull request
and fails if a cell here disagrees with it, so these digits cannot drift away
from the run they came from.

| chart | gesture | with a cube | without |
|---|---|---|---|
| binned density | zoom | **0.6 ms** (0.7) | 65.7 ms (76.9) |
| density | zoom | **2.6 ms** (3.4) | 80.6 ms (88.8) |
| density | brush | **5.1 ms** (5.8) | 82.0 ms (91.2) |
| binned density | brush | **0.7 ms** (0.8) | 74.2 ms (87.3) |
| raw scatter, two views | zoom | *no cube possible* | 167.0 ms (238.9) |

Both columns are measured in the same run, so the comparison is not confounded
by how busy the machine was on a given day.

The first gesture on a plot pays a one-time cost to build the cube — it shows up
as an outlier, not in the median. Every gesture after it reads the cube.

## When you do not get one, and what to do

**A raw scatter at scale.** Nothing can be pre-computed, so the cost is the
query plus drawing the points. If you are working with millions of rows, a binned
density says the same thing about shape and stays fast — and above roughly a
hundred thousand drawn points brightfield will draw a sample and
say so in the plot, because a picture that silently omits most of its data
is worse than a slow one.

**A near-unique filter column.** Filtering on something like a raw timestamp or
an ID gives a cube the size of the data. Rounding or binning that column first —
to a day, to a bucket — restores the speed, because it is the distinct count that
matters rather than the row count.

**A brush or slider moved while a zoom is held.** Today that combination falls
back to querying the source directly. It returns exactly the right answer; it is
just not accelerated. This is a known gap rather than a design decision.

## What is never traded

None of this changes what a query returns. A cube is a substitution the engine
makes only when it can prove the result is identical — it is matched against the
exact query that would otherwise run, and anything that does not match falls
through to the direct path. The failure mode of pre-aggregation here is a slow
answer rather than a wrong one, and
[`crates/brightfield-engine/tests/preagg_oracle.rs`](../crates/brightfield-engine/tests/preagg_oracle.rs)
is what holds that: it runs each substitution and the query it replaces against
DuckDB and compares the two answers.
