//! Gate: Structural test against the canonical `crossfilter.yaml`
//! spec from the vendored corpus. Verifies that:
//!
//! - `params.brush` is parsed as a `SelectionNode` (not a value param).
//! - The mark's `data.filterBy` is lifted to a `ParamRef`.
//! - The interactor's `as` option is lifted to a `ParamRef`.
//!
//! If this test fails, the cross-filtering contract that drives brightfield's
//! coordinator has broken.

use std::path::PathBuf;

use brightfield_spec::ast::MarkData;
use brightfield_spec::{
    parse_spec_path, Component, Mark, ParamNode, SelectionNode, ValueOrParamRef,
};

#[test]
fn crossfilter_structural() {
    let path = corpus_path("crossfilter.yaml");
    let out = parse_spec_path(&path).expect("crossfilter.yaml must parse");

    // 1. params.brush is a SelectionNode with select: crossfilter
    let brush = out
        .spec
        .params
        .get("brush")
        .expect("params.brush must be present");
    let sel: &SelectionNode = match brush {
        ParamNode::Selection(s) => s,
        ParamNode::Value(_) => panic!("params.brush must be a SelectionNode, got Value"),
    };
    assert_eq!(sel.select.wire_name(), "crossfilter");

    // 2. Walk the root and find the first Mark whose data.filterBy is a ParamRef.
    let root = out.spec.root.as_ref().expect("crossfilter has a root");
    let mut saw_filter_by = false;
    let mut saw_as = false;
    walk(root, &mut |c| match c {
        Component::Mark(Mark {
            data: Some(MarkData::From { filter_by, .. }),
            ..
        }) => {
            if let Some(r) = filter_by {
                assert_eq!(r.to_wire(), "$brush");
                saw_filter_by = true;
            }
        }
        Component::Interactor(i) => {
            if let Some(ValueOrParamRef::Param(r)) = i.options.get("as") {
                assert_eq!(r.to_wire(), "$brush");
                saw_as = true;
            }
        }
        _ => {}
    });

    assert!(
        saw_filter_by,
        "expected mark.data.filterBy to lift to ParamRef"
    );
    assert!(saw_as, "expected interactor.as to lift to ParamRef");
}

fn walk(c: &Component, f: &mut impl FnMut(&Component)) {
    f(c);
    match c {
        Component::Plot(p) => {
            for item in &p.items {
                walk(item, f);
            }
        }
        Component::HConcat(cc) | Component::VConcat(cc) => {
            for item in &cc.items {
                walk(item, f);
            }
        }
        _ => {}
    }
}

fn corpus_path(name: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .join("vendor")
        .join("mosaic-specs")
        .join("yaml")
        .join(name)
}
