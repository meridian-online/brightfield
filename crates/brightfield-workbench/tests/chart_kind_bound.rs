//! What a [`Bound`] guarantees to a builder, and where that guarantee stops.
//!
//! An external test crate on purpose. The claim under test is about what a
//! caller *outside* this crate can reach, so an in-crate test could not make
//! it: inside `brightfield-workbench`, `Bound`'s field is visible and the
//! question does not arise. Everything here goes through the published API.
//!
//! [`Bound`]: brightfield_workbench::registry::Bound

use brightfield_workbench::registry::{
    ChartKind, ChartKindId, Field, FieldSlot, FieldType, ModuleOptions,
};
use brightfield_workbench::Icon;

/// Bars takes a name on `x`. It is the constraint the forwarded call below
/// walks past, so it is declared once, here, and read by both tests.
const BAR_SLOTS: &[FieldSlot] = &[
    FieldSlot::required("x", &[FieldType::Categorical]),
    FieldSlot::required("y", &[FieldType::Quantitative]),
];

/// Two measures — what `bars` will not take on `x`.
const TWO_MEASURES: &[FieldSlot] = &[
    FieldSlot::required("x", &[FieldType::Quantitative]),
    FieldSlot::required("y", &[FieldType::Quantitative]),
];

/// The spec carries the bound column names verbatim, so an assertion can read
/// which column reached which slot rather than only that a spec was produced.
fn bars() -> ChartKind<String> {
    ChartKind {
        id: ChartKindId::new("bars"),
        icon: Icon("chart-bar"),
        description: "Ranks a category by a measure",
        slots: BAR_SLOTS,
        controls: Vec::new,
        build: |binding, _| format!("bar x={:?} y={:?}", binding.name("x"), binding.name("y")),
    }
}

/// `bars` under its own id, declaring different slots — a stale copy, or a
/// second registry's divergent declaration.
///
/// Nothing in the type system stops this: `ChartKindId` wraps a `&'static
/// str`. `ChartKindRegistry::new` rejects a duplicate id *within one registry*,
/// which is why the three below are built outside one.
fn stale_bars() -> ChartKind<String> {
    ChartKind {
        slots: TWO_MEASURES,
        ..bars()
    }
}

/// A role `bars` has never declared.
const COLOUR_ONLY: &[FieldSlot] = &[FieldSlot::required("colour", &[FieldType::Categorical])];

/// `bars`' `x` and not its `y`.
const NAME_ONLY: &[FieldSlot] = &[FieldSlot::required("x", &[FieldType::Categorical])];

/// The same id, declaring a role `bars` has never had.
fn colour_bars() -> ChartKind<String> {
    ChartKind {
        slots: COLOUR_ONLY,
        ..bars()
    }
}

/// The same id, declaring `bars`' `x` and not its `y` — `bars` as it was
/// before the measure was required.
fn x_only_bars() -> ChartKind<String> {
    ChartKind {
        slots: NAME_ONLY,
        ..bars()
    }
}

/// A kind that declares its own slots and then hands the binding to another
/// kind's builder.
fn forwarder() -> ChartKind<String> {
    ChartKind {
        id: ChartKindId::new("forwarder"),
        icon: Icon("chart-dots"),
        description: "Builds through another kind's builder",
        slots: TWO_MEASURES,
        controls: Vec::new,
        build: |bound, options| (bars().build)(bound, options),
    }
}

fn category(name: &str) -> Field {
    Field::new(name, FieldType::Categorical)
}

fn measure(name: &str) -> Field {
    Field::new(name, FieldType::Quantitative)
}

/// The guarantee, from outside the crate: a builder is reachable only through
/// `spec`, and `spec` refuses a binding another kind made.
///
/// The compile-time half — that `(kind.build)(&binding, ..)` will not take a
/// bare `FieldBinding` — is a `compile_fail` doctest on `Bound` itself, since
/// a non-compiling line cannot live in a test that must build.
#[test]
fn a_builder_is_reachable_only_with_a_binding_this_kind_checked() {
    let binding = forwarder()
        .bind(&[measure("revenue"), measure("margin")])
        .expect("two measures fill forwarder's slots");

    let err = bars()
        .spec(&binding, &ModuleOptions::default())
        .expect_err("bars did not make this binding");
    assert!(
        err.contains("forwarder"),
        "the error names the kind that made it: {err}"
    );

    let own = bars()
        .bind(&[category("sector"), measure("revenue")])
        .expect("a name and a measure fill bars' slots");
    assert_eq!(
        bars()
            .spec(&own, &ModuleOptions::default())
            .expect("bars made this binding"),
        r#"bar x=Some("sector") y=Some("revenue")"#
    );
}

/// Sharing an id does not mean declaring the same slots, and `spec` does not
/// treat it as if it did.
///
/// `stale_bars` takes a measure on `x`; `bars` takes a name. Both answer to
/// `"bars"`, so the id check cannot tell them apart — it is the slot re-check
/// that refuses the column. Without it a quantitative column lands in a
/// categorical-only slot and the builder, which has no error path by design,
/// draws it.
#[test]
fn a_binding_from_a_kind_sharing_this_id_is_checked_against_these_slots() {
    let binding = stale_bars()
        .bind(&[measure("revenue"), measure("margin")])
        .expect("two measures fill the stale declaration's slots");
    assert_eq!(
        binding.kind(),
        ChartKindId::new("bars"),
        "the id check has nothing to bite on"
    );

    let err = bars()
        .spec(&binding, &ModuleOptions::default())
        .expect_err("bars declares x Categorical and was handed a measure");
    assert!(
        err.contains("revenue") && err.contains('x'),
        "the error names the column and the slot it could not fill: {err}"
    );
}

/// The same re-check on shape rather than on type: a role this kind does not
/// declare, and a required slot nothing filled.
///
/// Two ways a binding can share `bars`' id and still not fit it. Every column
/// in the second one is a column `bars` would take — it is the absent `y` that
/// makes it wrong, which is why the type loop alone would let it through.
#[test]
fn a_binding_sharing_this_id_must_also_match_this_kinds_shape() {
    let undeclared = colour_bars()
        .bind(&[category("sector")])
        .expect("one name fills colour_bars' one slot");
    let err = bars()
        .spec(&undeclared, &ModuleOptions::default())
        .expect_err("bars has no colour slot");
    assert!(
        err.contains("colour"),
        "the error names the undeclared role: {err}"
    );

    let short = x_only_bars()
        .bind(&[category("sector")])
        .expect("one name fills x_only_bars' one slot");
    assert_eq!(short.name("x"), Some("sector"), "and x itself is fine");
    let err = bars()
        .spec(&short, &ModuleOptions::default())
        .expect_err("bars requires a y and this binding has none");
    assert!(
        err.contains('y'),
        "the error names the unfilled required slot: {err}"
    );
}

/// The boundary: a forwarded `Bound` carries the forwarder's slots, not the
/// receiver's.
///
/// `bars` takes only a `Categorical` on `x`. Here its builder runs over a
/// `Quantitative` one and says so, because the binding was checked against
/// `forwarder` — which declared `x` as a measure — and nothing re-checks it at
/// the hand-off. This is asserted rather than fixed: a `fn` pointer carries no
/// kind identity, so no check in this crate can tell a forwarded call from a
/// direct one. It is pinned so that a later change closing it has to come
/// through here and update what `Bound`'s rustdoc promises.
#[test]
fn a_forwarded_bound_carries_the_forwarders_slots_not_the_receivers() {
    let binding = forwarder()
        .bind(&[measure("revenue"), measure("margin")])
        .expect("two measures fill forwarder's slots");
    assert_eq!(binding.kind(), ChartKindId::new("forwarder"));

    let spec = forwarder()
        .spec(&binding, &ModuleOptions::default())
        .expect("forwarder made this binding");
    assert_eq!(
        spec, r#"bar x=Some("revenue") y=Some("margin")"#,
        "bars' builder ran with a measure in a slot bars declares Categorical"
    );

    assert!(
        !bars().accepts(&[measure("revenue"), measure("margin")]),
        "and bars itself would not have bound those columns"
    );
}
