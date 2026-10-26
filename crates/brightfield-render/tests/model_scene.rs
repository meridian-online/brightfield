//! The Ossie model scene, driven through a real GPU.
//!
//! The retail fixture is drawn in both modes and compared with a committed PNG;
//! the selection and hit-test checks read pixels and geometry directly. Needs a GPU
//! adapter, as the other render suites do. Baselines are written only when
//! `UPDATE_SNAPSHOTS=1` is set.

use std::path::PathBuf;

use brightfield_ossie::{layout, read, Model, ModelLayout};
use brightfield_render::ink::ink;
use brightfield_render::model_scene::{hit_test, render_model, ModelFocus, Selection};
use brightfield_render::vello_renderer::VelloRenderer;
use vello::Scene;

fn retail() -> Model {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-ossie/fixtures/retail_extract/model.ossie.yaml");
    read(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn size(l: &ModelLayout) -> (u32, u32) {
    (l.width.ceil() as u32, l.height.ceil() as u32)
}

fn render(model: &Model, l: &ModelLayout, focus: &ModelFocus, dark: bool) -> Vec<u8> {
    let mut scene = Scene::new();
    render_model(&mut scene, model, l, focus, dark);
    let (w, h) = size(l);
    let canvas = ink(meridian_design::semantic(dark).surfaces.app);
    VelloRenderer::new()
        .lock()
        .unwrap()
        .render_to_pixels_with_base(&scene, w, h, canvas)
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.png"))
}

/// Compare with the committed PNG. A GPU and its driver may differ in the last bits of
/// antialiasing, so a pixel only counts as different past a few levels, and the frame
/// fails when more than half a percent of its pixels do.
fn assert_snapshot(name: &str, pixels: &[u8], (w, h): (u32, u32)) {
    let path = snapshot_path(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        image::save_buffer(&path, pixels, w, h, image::ColorType::Rgba8).unwrap();
        return;
    }
    let want = image::open(&path)
        .unwrap_or_else(|e| {
            panic!(
                "no baseline at {}: {e}; run with UPDATE_SNAPSHOTS=1 to write it",
                path.display()
            )
        })
        .to_rgba8();
    assert_eq!(
        (want.width(), want.height()),
        (w, h),
        "{name}: size changed"
    );
    let differing = want
        .as_raw()
        .chunks_exact(4)
        .zip(pixels.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 6))
        .count();
    let limit = (w as usize * h as usize) / 200;
    assert!(
        differing <= limit,
        "{name}: {differing} pixels differ (limit {limit})"
    );
}

#[test]
fn retail_baseline_light() {
    let (model, none) = (retail(), ModelFocus::default());
    let l = layout(&model);
    assert_snapshot(
        "model_retail_light",
        &render(&model, &l, &none, false),
        size(&l),
    );
}

#[test]
fn retail_baseline_dark() {
    let (model, none) = (retail(), ModelFocus::default());
    let l = layout(&model);
    assert_snapshot(
        "model_retail_dark",
        &render(&model, &l, &none, true),
        size(&l),
    );
}

fn pixel(pixels: &[u8], width: u32, x: f64, y: f64) -> [u8; 4] {
    let i = (y as usize * width as usize + x as usize) * 4;
    pixels[i..i + 4].try_into().unwrap()
}

#[test]
fn selection_dims_unconnected() {
    let model = retail();
    let l = layout(&model);
    let focus = ModelFocus {
        selected: Some(Selection::Relationship("transactions_products".into())),
    };
    let (w, _) = size(&l);
    for dark in [false, true] {
        let pixels = render(&model, &l, &focus, dark);
        let dimmed = ink(meridian_design::semantic(dark).text.disabled)
            .to_rgba8()
            .to_u8_array();
        let centre = |name: &str| {
            let c = l.cards.iter().find(|c| c.dataset == name).unwrap();
            pixel(&pixels, w, c.x + c.width / 2.0, c.y + c.height / 2.0)
        };
        // Loyalty visits joins neither end of the selected join.
        assert_eq!(centre("loyalty_visits"), dimmed, "dark={dark}");
        // Transactions is one end of it and keeps its own fill.
        assert_ne!(centre("transactions"), dimmed, "dark={dark}");
    }
}

#[test]
fn hit_test_finds_card_and_edge_label() {
    let model = retail();
    let l = layout(&model);
    let c = l.cards.iter().find(|c| c.dataset == "stores").unwrap();
    assert_eq!(
        hit_test(&l, c.x + c.width / 2.0, c.y + 4.0),
        Some(Selection::Dataset("stores".into()))
    );
    // Every join's label can be picked somewhere on the canvas.
    for e in &l.edges {
        let want = Selection::Relationship(e.relationship.clone());
        // Stepped finer than a pill is tall, so a visible pill cannot be skipped.
        let found = (0..l.height as i32 / 6)
            .flat_map(|y| (0..l.width as i32 / 6).map(move |x| (x * 6, y * 6)))
            .any(|(x, y)| hit_test(&l, f64::from(x), f64::from(y)).as_ref() == Some(&want));
        assert!(found, "no point selects {}", e.relationship);
    }
    assert_eq!(hit_test(&l, -5.0, -5.0), None);
}
