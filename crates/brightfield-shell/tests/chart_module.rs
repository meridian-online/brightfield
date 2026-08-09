//! The chart pane draws through the chart-kind registry — asserted by taking
//! the registry away.
//!
//! The claim these tests hold is not "a registry exists" or "a `ChartModule` is
//! constructed somewhere". It is that the registry is **the path the picture
//! travels**: remove the kind a document's picture was chosen as, and the
//! picture stops reaching the screen and the pane says which kind is missing.
//!
//! # Why the assertions are on the raster's rect
//!
//! These run headless, so there is no wgpu device and the raster is a blank
//! allocation rather than pixels — but the *layout* happens either way (see
//! [`brightfield_shell::app::ChartDoc::present_raster`]), and the rect is
//! recorded on the document by the one routine that puts the picture on screen.
//! So `raster_rect` is `Some` exactly when the picture was drawn, and it is the
//! observable a GPU-free machine can hold. The pixel tier's baselines are what
//! say the pixels are unchanged; this tier says the picture is drawn at all,
//! and by which route.
//!
//! # The pair, and why it takes both
//!
//! `the_pane_draws_a_registry_authored_picture` fails if the kind leaves the
//! registry. `a_kind_this_build_does_not_have_stops_the_picture` fails if the
//! pane ever stops consulting the registry and draws regardless. Either test
//! alone is satisfiable by a build that ignores the registry in one direction.

use brightfield_shell::app::{Authored, ChartDoc};
use brightfield_shell::chart_item::ChartItem;
use brightfield_shell::chart_kinds;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use brightfield_shell::window::{chart_window_size, Boot, MeridianApp};
use brightfield_workbench::item::{ChartModule, ModuleHost};
use brightfield_workbench::registry::{ChartKindId, Field, FieldType};
use brightfield_workbench::Item;

/// The table the fixtures are drawn over, declared under the name every kind in
/// the registry reads. Inline rows rather than a file, so no DuckDB reader and
/// no temporary directory is involved in a test about which chart is drawn.
const ROWS: &str = "\
data:
  opened:
    - { amount: 1, region: north }
    - { amount: 4, region: north }
    - { amount: 9, region: south }
    - { amount: 16, region: east }
    - { amount: 25, region: west }
";

/// The document a chart kind chose, composed: the block that kind built over
/// [`ROWS`], and the [`Authored`] record the pane re-makes its module from.
///
/// Both halves come out of the registry — the block from the kind's own
/// builder, the fields from the same list that was bound to it — so a fixture
/// cannot pin a picture the product would not have produced.
fn authored_by(kind_id: ChartKindId, fields: Vec<Field>) -> (Composed, Authored) {
    let kind = chart_kinds::find(kind_id).expect("the kind is in this build");
    let binding = kind.bind(&fields).expect("the columns fill its slots");
    let block = kind
        .spec(&binding, &kind.options())
        .expect("the kind builds its spec");
    let source = format!("{ROWS}{block}width: 400\nheight: 300\n");
    let composed =
        compose_spec_str(&source, None).unwrap_or_else(|e| panic!("{kind_id}: {e}\n{source}"));
    (
        composed,
        Authored {
            kind: kind_id,
            fields,
            block,
        },
    )
}

fn measure(name: &str) -> Field {
    Field::new(name, FieldType::Quantitative)
}

/// One headless layout pass at the window the shell would ask for, over a
/// document that carries `authored`. Returns the document's recorded rects.
fn laid_out(composed: Composed, authored: Option<Authored>) -> (Option<egui::Rect>, Option<egui::Rect>) {
    let (w, h) = chart_window_size(&composed);
    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
    if let Some(authored) = authored {
        app.chart_doc_mut().set_authored(authored);
    }
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    let doc = app.chart_doc();
    (doc.raster_rect, doc.legend_rect)
}

/// **A picture a chart kind chose is drawn, and drawn through that kind's
/// module.**
///
/// The pane holds no branch on the kind: it asks the document which kind chose
/// the picture, looks that kind up in the registry, and draws the
/// `ChartModule` the two make. Delete the kind's entry from
/// `chart_kinds::registry` and this test goes red — which is the whole of what
/// "the registry is the path" means.
#[test]
fn the_pane_draws_a_registry_authored_picture() {
    for (kind_id, fields) in [
        (chart_kinds::BINNED_HISTOGRAM, vec![measure("amount")]),
        (
            chart_kinds::COUNT_GRID,
            vec![
                Field::new("region", FieldType::Categorical),
                Field::new("amount", FieldType::Categorical),
            ],
        ),
    ] {
        let (composed, authored) = authored_by(kind_id, fields);
        let (width, height) = (composed.width, composed.height);
        let (raster, _legend) = laid_out(composed, Some(authored));
        let raster = raster.unwrap_or_else(|| {
            panic!("{kind_id}: the module drew no raster — the picture never reached the pane")
        });
        assert!(
            (raster.width() - width as f32).abs() < 0.5
                && (raster.height() - height as f32).abs() < 0.5,
            "{kind_id}: the raster was reserved at {raster:?}, and the \
             dashboard composed at {width}x{height}"
        );
    }
}

/// **A kind this build does not have stops the picture**, and the pane says
/// which kind is missing rather than drawing a header over a blank rect.
///
/// The document is the same one the test above draws — same spec, same
/// composition, same rows. The only difference is the kind it claims to have
/// been chosen as, and that difference is the whole of what stops it.
#[test]
fn a_kind_this_build_does_not_have_stops_the_picture() {
    let stranger = ChartKindId::new("sunburst");
    assert!(
        chart_kinds::find(stranger).is_none(),
        "the fixture's premise is that this build has no such kind"
    );

    let (composed, mut authored) = authored_by(chart_kinds::BINNED_HISTOGRAM, vec![measure("amount")]);
    authored.kind = stranger;

    let (raster, legend) = laid_out(composed, Some(authored.clone()));
    assert_eq!(
        raster, None,
        "a picture whose kind left the registry was drawn anyway"
    );
    assert_eq!(legend, None, "and its legend band was reserved anyway");

    // …and what the reader is told. The pane's empty state names the missing
    // kind, so the answer is actionable rather than a blank pane.
    let mut doc = ChartDoc::headless(Composed::empty());
    doc.set_authored(authored);
    doc.composed = compose_spec_str(
        &format!("{ROWS}plot:\n  - mark: dot\n    data: {{ from: opened }}\n    x: 'amount'\n    y: 'amount'\nwidth: 400\nheight: 300\n"),
        None,
    )
    .expect("the fixture composes");
    let empty = Item::subject(&ChartItem::new(), &doc)
        .empty_state
        .expect("a document whose kind is missing has nothing to draw");
    assert_eq!(empty.headline, "This build has no chart of that kind");
    assert!(
        empty.body.contains("sunburst"),
        "the body names the missing kind: {}",
        empty.body
    );
}

/// **The document refuses to present a picture the module did not ask for.**
///
/// A module's spec is rebuilt from its kind and its columns every frame; the
/// picture on screen was composed once. Hand the module a different column and
/// its spec stops being the one the document holds — and the document draws
/// nothing rather than putting one chart under another chart's module.
///
/// Unreachable from the running binary today, and that is stated where the
/// check lives: no shipped kind declares a control and nothing rebinds a
/// module's columns, so the block a module builds is a function of the document
/// it was built from. It is a defence against a future edit, so a test is the
/// only thing that can hold it.
#[test]
fn a_module_asking_for_a_different_picture_is_not_handed_this_one() {
    for (whose, fields, want_drawn) in [
        ("the document's own columns", vec![measure("amount")], true),
        (
            "a column it was never composed for",
            vec![measure("other")],
            false,
        ),
    ] {
        let (composed, authored) =
            authored_by(chart_kinds::BINNED_HISTOGRAM, vec![measure("amount")]);
        let kind = chart_kinds::find(authored.kind).expect("shipped");
        let mut doc = ChartDoc::headless(composed);
        doc.set_authored(authored);
        let mut module = ChartModule::new(
            brightfield_shell::app::CHART,
            "Chart",
            kind,
            fields,
        );

        let ctx = egui::Context::default();
        let mut requests = Vec::new();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| {
                let mut cx = brightfield_workbench::ItemCtx::new(
                    Mode::Light,
                    brightfield_workbench::PaneKey::new(
                        brightfield_workbench::ViewKind::Charts,
                        brightfield_shell::app::CHART,
                    ),
                    egui_tiles::TileId::from_u64(1),
                    false,
                    &mut requests,
                );
                Item::ui(&mut module, &mut doc, ui, &mut cx);
            },
        );

        assert_eq!(
            doc.raster_rect.is_some(),
            want_drawn,
            "{whose}: the document presented {} raster",
            if want_drawn { "no" } else { "a" }
        );
    }
}

/// The registry the pane reads is the one the document hands it — not a second
/// list the pane keeps.
///
/// Cheap, and it is the join the two tests above depend on: if `ChartDoc`'s
/// `chart_kinds` ever answered with something other than
/// `chart_kinds::registry`, removing a kind from the shipped registry would
/// stop reddening them and nothing else would notice.
#[test]
fn the_document_hands_the_pane_the_shipped_registry() {
    let doc = ChartDoc::headless(Composed::empty());
    assert_eq!(doc.chart_kinds().ids(), chart_kinds::registry().ids());
}
