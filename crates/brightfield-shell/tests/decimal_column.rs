//! **A `DECIMAL` column draws the picture the `DOUBLE` it casts to draws.**
//!
//! DuckDB types the product of a column and a decimal literal (`x * 10.0`) as
//! `DECIMAL` and hands it over as Arrow `Decimal128`. Each f64 reader in
//! brightfield-render carries a `Decimal128` arm, pinned in that crate against
//! a hand-built `DOUBLE` twin. This file runs the real path instead — a SQL
//! step through DuckDB, the engine and the composer — and compares what it
//! draws with the same step cast to `DOUBLE` by DuckDB itself. That the column
//! arrives as `Decimal128` at all is brightfield-engine's
//! `a_decimal_column_reaches_the_renderer_as_decimal128`.

use std::path::PathBuf;

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::{compose_spec_sampled, Composed};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bf-decimal-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn compose(dir: &std::path::Path, name: &str, yaml: &str) -> Composed {
    let spec = dir.join(format!("{name}.yaml"));
    std::fs::write(&spec, yaml).expect("write spec");
    compose_spec_sampled(spec.to_str().unwrap(), None)
        .unwrap_or_else(|e| panic!("{name}: the spec composes: {e}"))
}

fn linear_domain(c: &Composed, channel: Channel) -> (f64, f64) {
    match c.plots[0].scales.get(channel) {
        Some(Scale::Linear {
            domain_min,
            domain_max,
            ..
        }) => (*domain_min, *domain_max),
        other => panic!("expected a linear {channel:?} scale, got {other:?}"),
    }
}

/// One dot plot over a SQL step whose `x` and `y` are the expressions given.
fn step_spec(x: &str, y: &str) -> String {
    format!(
        "data:\n  points:\n    query: |\n      SELECT {x} AS x, {y} AS y FROM range(48) AS t(i)\n\
         plot:\n  - mark: dot\n    data: {{ from: points }}\n    x: x\n    y: y\n\
         width: 400\nheight: 300\n"
    )
}

/// **A `DECIMAL` step draws the axis domains and the marks its `DOUBLE` cast
/// draws.** The domains are compared as numbers and the two pictures pixel for
/// pixel, so a plot that keeps the axis but loses the marks under it fails on
/// the second comparison after passing the first.
#[test]
fn a_decimal_step_draws_the_picture_its_double_cast_draws() {
    let dir = scratch("step");
    let (x, y) = ("i * 10.0", "(i * 7 % 13) * 2.5 - 4.25");
    let decimal = compose(&dir, "decimal", &step_spec(x, y));
    let double = compose(
        &dir,
        "double",
        &step_spec(
            &format!("CAST({x} AS DOUBLE)"),
            &format!("CAST({y} AS DOUBLE)"),
        ),
    );

    assert_eq!(
        linear_domain(&double, Channel::X),
        (0.0, 470.0),
        "fixture check"
    );
    assert_eq!(
        linear_domain(&double, Channel::Y),
        (-4.25, 25.75),
        "fixture check"
    );
    for ch in [Channel::X, Channel::Y] {
        assert_eq!(
            linear_domain(&decimal, ch),
            linear_domain(&double, ch),
            "the DECIMAL step's {ch:?} domain must be its DOUBLE cast's"
        );
    }

    let (decimal_png, double_png) = (dir.join("decimal.png"), dir.join("double.png"));
    capture_vello_only(decimal, 1.0, &decimal_png).expect("capture the DECIMAL plot");
    capture_vello_only(double, 1.0, &double_png).expect("capture the DOUBLE plot");
    let decimal_img = image::open(&decimal_png).expect("open png").to_rgba8();
    let double_img = image::open(&double_png).expect("open png").to_rgba8();
    assert_eq!(decimal_img.dimensions(), double_img.dimensions());
    let differing = decimal_img
        .pixels()
        .zip(double_img.pixels())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing,
        0,
        "the DECIMAL plot differs from its DOUBLE cast in {differing} pixels; see {} and {}",
        decimal_png.display(),
        double_png.display()
    );
}

/// **A `DECIMAL` too wide for a double to hold exactly lands on the double
/// DuckDB casts it to.** `692721592851106.19` at scale 2 is past `2^53`
/// unscaled, where dividing once by `10^2` lands on a neighbour of the double
/// DuckDB's cast returns; the fixture check holds the value to that case.
#[test]
fn a_wide_decimal_lands_on_the_double_duckdb_casts_it_to() {
    let dir = scratch("wide");
    let v = "CAST(692721592851106.19 - i AS DECIMAL(18, 2))";
    let decimal = compose(&dir, "decimal", &step_spec(v, v));
    let double = compose(
        &dir,
        "double",
        &step_spec(
            &format!("CAST({v} AS DOUBLE)"),
            &format!("CAST({v} AS DOUBLE)"),
        ),
    );

    let (_, max) = linear_domain(&double, Channel::X);
    assert_ne!(
        max,
        69_272_159_285_110_619_i64 as f64 / 100.0,
        "fixture check: one division must land elsewhere for this value"
    );
    for ch in [Channel::X, Channel::Y] {
        assert_eq!(
            linear_domain(&decimal, ch),
            linear_domain(&double, ch),
            "the DECIMAL step's {ch:?} domain must be its DOUBLE cast's"
        );
    }
}

/// **`examples/geo.yaml` fills its regions through the sequential ramp**,
/// built from its `DECIMAL` rates rather than the flat default fill. The
/// example is the committed file whose picture reads a `DECIMAL` column; that
/// its rates arrive as one is brightfield-engine's witness.
#[test]
fn the_geo_example_fills_its_decimal_rates_through_the_ramp() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/geo.yaml");
    let composed = compose_spec_sampled(path, None).expect("examples/geo.yaml composes");
    match composed.plots[0].scales.get(Channel::Fill) {
        Some(Scale::Sequential {
            domain_min,
            domain_max,
            ..
        }) => assert_eq!((*domain_min, *domain_max), (0.0, 9.5)),
        other => panic!("expected the sequential fill ramp, got {other:?}"),
    }
}
