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
