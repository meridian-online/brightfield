//! MenuElement — thin GPUI shim for the menu widget family.
//!
//! The model half (`MenuBinding` / `MenuState` / `commit_menu_release`)
//! lives in `menu.rs` and stays gpui-free; this file is the window layer,
//! mirroring the `slider.rs` / `slider_element.rs` split. It contains NO
//! param or option-resolution logic — every decision is a call into the
//! model, and a commit routes through the coordinator
//! (`commit_menu` → `propagate_param` → re-render subscribers).
//!
//! Built from GPUI primitives (divs / text) only — no gpui-component
//! widgets ('hand-drawn' constrains the dependency, not the primitive; the
//! egui-exit posture). The open menu's option list rides [`gpui::deferred`]
//! so it paints ABOVE sibling plots/widgets/legends, and `occlude()` so a
//! click on an option can never fall through to a chart's brush handler
//! beneath.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    deferred, div, px, AnyElement, App, Entity, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, Window,
};

use brightfield_spec::ast::SpecValue;
use brightfield_spec::layout::RADIO_ROW_HEIGHT;

use crate::chart_view::PlacedMenu;
use crate::crossfilter::CrossfilterCoordinator;
use crate::menu::{
    checkbox_checked, checkbox_toggle_index, option_label, MenuBinding, MenuState, MenuStyle,
};
use crate::theme_bridge::rgba;

/// Widget ink — kept in sync with the vello resting twin
/// (brightfield-render scene.rs `WIDGET_*` constants) so the dumped PNG
/// matches the window (the SLIDER_* convention). Both sides
/// read the same Meridian tokens: fill = chart surface, border = warm gray
/// step 5 (the slider-track gray), label = primary ink, affordance
/// (chevron / ring outline) = muted ink, active (selected dot / check) =
/// Maritime focus ink.
const WIDGET_FILL_TOKEN: meridian_design::colour::Rgba = meridian_design::chrome::INK_LIGHT.surface;
const WIDGET_BORDER_TOKEN: meridian_design::colour::Rgba = meridian_design::scales::GRAY_LIGHT[4];
const WIDGET_LABEL_TOKEN: meridian_design::colour::Rgba =
    meridian_design::chrome::INK_LIGHT.ink_primary;
const WIDGET_AFFORDANCE_TOKEN: meridian_design::colour::Rgba =
    meridian_design::chrome::INK_LIGHT.ink_muted;
const WIDGET_ACTIVE_TOKEN: meridian_design::colour::Rgba = meridian_design::chrome::INK_LIGHT.focus;
/// Hover wash for an open menu's option rows — GPUI-side only (never in the
/// Vello scene, so PNGs are untouched): warm gray step 2.
const WIDGET_HOVER_TOKEN: meridian_design::colour::Rgba = meridian_design::scales::GRAY_LIGHT[1];

/// Height of one option row in the open menu's popup list (logical px).
const MENU_OPTION_ROW_HEIGHT: f64 = 24.0;
/// Widget label/text size (logical px) — matches the twin's label size.
const WIDGET_TEXT_SIZE: f32 = 12.0;

/// Persistent per-menu UI state held in a gpui `Entity`: the committed
/// (resting) value and the interaction state. Binding + geometry live on
/// the [`PlacedMenu`], rebuilt each frame from the host placement.
pub struct MenuWidget {
    /// The current committed value — what the closed box / checked state /
    /// selected radio row displays.
    pub value: SpecValue,
    /// The interaction state (`Closed` at rest).
    pub gesture: MenuState,
}

impl MenuWidget {
    /// A widget resting at `value`.
    pub fn new(value: SpecValue) -> Self {
        Self {
            value,
            gesture: MenuState::Closed,
        }
    }
}

/// Build the menu-family element for one placed widget. `index` is the
/// widget's index into the coordinator's menu bindings (menus hosted in
/// order — the registration-order contract); `dashboard_height`
/// decides whether an open menu's list drops below the box or opens upward
/// at the dashboard's bottom edge (cosmetic, per the spec's open question).
pub fn menu_element(
    placed: &PlacedMenu,
    index: usize,
    dashboard_height: f64,
    cx: &App,
) -> AnyElement {
    match placed.binding.style {
        MenuStyle::Menu => menu_presentation(placed, index, dashboard_height, cx).into_any_element(),
        MenuStyle::Radio => radio_presentation(placed, index, cx).into_any_element(),
        MenuStyle::Checkbox => checkbox_presentation(placed, index, cx).into_any_element(),
    }
}

/// Commit option `option_index` on widget `menu_index`: run the model's
/// pick transition, route the `Committed` state through the coordinator
/// (re-query + re-render subscribers), and settle the widget at the picked
/// value. The dispatch decision (same-value no-op included) lives in the
/// model's `commit_menu_release`, reached via `commit_menu` against the
/// session's live param value — this shim only forwards.
fn commit_pick(
    state: &Entity<MenuWidget>,
    coordinator: &Option<Rc<RefCell<CrossfilterCoordinator>>>,
    binding: &MenuBinding,
    menu_index: usize,
    option_index: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let next = state.read(cx).gesture.pick(option_index);
    if let MenuState::Committed { index: picked } = next {
        if let Some(coord) = coordinator {
            coord
                .borrow_mut()
                .commit_menu(menu_index, &MenuState::Committed { index: picked }, cx);
        }
        state.update(cx, |w, _| {
            if let Some(v) = binding.options.get(picked) {
                w.value = v.clone();
            }
            w.gesture = MenuState::Closed;
        });
        window.refresh();
    }
}

/// `style: menu` — a closed box showing the current value + a chevron; a
/// click toggles the option list open (deferred + occluding); an option
/// click commits; a click away closes without dispatch.
fn menu_presentation(
    placed: &PlacedMenu,
    index: usize,
    dashboard_height: f64,
    cx: &App,
) -> impl IntoElement {
    let widget = placed.state.read(cx);
    let open = matches!(widget.gesture, MenuState::Open { .. });
    let current = widget.value.clone();
    let binding = placed.binding.clone();

    // The closed box: current value left, chevron right.
    let box_row = div()
        .id(SharedString::from(format!("bf-menu-{index}")))
        .size_full()
        .flex()
        .items_center()
        .justify_between()
        .px(px(10.0))
        .bg(rgba(WIDGET_FILL_TOKEN))
        .border_1()
        .border_color(rgba(WIDGET_BORDER_TOKEN))
        .rounded(px(4.0))
        .text_size(px(WIDGET_TEXT_SIZE))
        .text_color(rgba(WIDGET_LABEL_TOKEN))
        .cursor_pointer()
        .child(option_label(&current))
        .child(
            div()
                .text_color(rgba(WIDGET_AFFORDANCE_TOKEN))
                .child(if open { "▴" } else { "▾" }),
        )
        .on_mouse_down(MouseButton::Left, {
            let state = placed.state.clone();
            // Decide from the RENDER-TIME open snapshot, never a re-read of
            // the live gesture: one physical click on the OPEN menu's box
            // double-fires — gpui runs the popup's `on_mouse_down_out` in
            // the CAPTURE phase (click_away → Closed) BEFORE this BUBBLE
            // handler, so a `toggle_open()` re-read of the now-Closed state
            // would always re-open, killing the toggle-to-close half of
            // the ▴ chevron advertises. Pinned headlessly by
            // menu.rs `composed_box_click_closes_after_capture_click_away`.
            let was_open = open;
            move |_ev, window, cx| {
                state.update(cx, |w, _| {
                    w.gesture = if was_open {
                        MenuState::Closed
                    } else {
                        w.gesture.toggle_open()
                    };
                });
                window.refresh();
            }
        });

    // The open option list. Deferred → paints above every non-deferred
    // sibling; occlude → hitboxes beneath (a chart's brush) never see the
    // click. Opens upward when a downward list would leave the dashboard.
    let popup_height = binding.options.len() as f64 * MENU_OPTION_ROW_HEIGHT + 2.0;
    let open_up = placed.y + placed.height + popup_height > dashboard_height;
    let popup = open.then(|| {
        deferred(
            div()
                .id(SharedString::from(format!("bf-menu-popup-{index}")))
                .occlude()
                .absolute()
                .left_0()
                .w(px(placed.width as f32))
                .when(open_up, |el| el.bottom(px(placed.height as f32)))
                .when(!open_up, |el| el.top(px(placed.height as f32)))
                .bg(rgba(WIDGET_FILL_TOKEN))
                .border_1()
                .border_color(rgba(WIDGET_BORDER_TOKEN))
                .rounded(px(4.0))
                .text_size(px(WIDGET_TEXT_SIZE))
                .text_color(rgba(WIDGET_LABEL_TOKEN))
                .children(binding.options.iter().enumerate().map(|(i, opt)| {
                    let selected = *opt == current;
                    div()
                        .id(SharedString::from(format!("bf-menu-{index}-opt-{i}")))
                        .h(px(MENU_OPTION_ROW_HEIGHT as f32))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(WIDGET_HOVER_TOKEN)))
                        .when(selected, |el| el.text_color(rgba(WIDGET_ACTIVE_TOKEN)))
                        .child(option_label(opt))
                        .on_mouse_down(MouseButton::Left, {
                            let state = placed.state.clone();
                            let coordinator = placed.coordinator.clone();
                            let binding = binding.clone();
                            move |_ev, window, cx| {
                                commit_pick(&state, &coordinator, &binding, index, i, window, cx);
                            }
                        })
                }))
                // Click-away closes WITHOUT dispatch (model transition).
                .on_mouse_down_out({
                    let state = placed.state.clone();
                    move |_ev, window, cx| {
                        state.update(cx, |w, _| w.gesture = w.gesture.click_away());
                        window.refresh();
                    }
                }),
        )
    });

    div()
        .relative()
        .size_full()
        .child(box_row)
        .children(popup)
}

/// `style: radio` — all options rendered expanded (one 22px row each);
/// an option click commits directly. The selected row shows a filled dot.
fn radio_presentation(placed: &PlacedMenu, index: usize, cx: &App) -> impl IntoElement {
    let current = placed.state.read(cx).value.clone();
    let binding = placed.binding.clone();

    div()
        .size_full()
        .flex()
        .flex_col()
        .justify_center()
        .px(px(4.0))
        .text_size(px(WIDGET_TEXT_SIZE))
        .text_color(rgba(WIDGET_LABEL_TOKEN))
        .children(binding.options.iter().enumerate().map(|(i, opt)| {
            let selected = *opt == current;
            div()
                .id(SharedString::from(format!("bf-radio-{index}-opt-{i}")))
                .h(px(RADIO_ROW_HEIGHT as f32))
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(
                    // The ring, with a filled Maritime dot when selected.
                    div()
                        .w(px(12.0))
                        .h(px(12.0))
                        .rounded_full()
                        .bg(rgba(WIDGET_FILL_TOKEN))
                        .border_1()
                        .border_color(rgba(WIDGET_AFFORDANCE_TOKEN))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |el| {
                            el.child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded_full()
                                    .bg(rgba(WIDGET_ACTIVE_TOKEN)),
                            )
                        }),
                )
                .child(option_label(opt))
                .on_mouse_down(MouseButton::Left, {
                    let state = placed.state.clone();
                    let coordinator = placed.coordinator.clone();
                    let binding = binding.clone();
                    move |_ev, window, cx| {
                        commit_pick(&state, &coordinator, &binding, index, i, window, cx);
                    }
                })
        }))
}

/// `style: checkbox` — a box + check glyph + label (the bound param's
/// name); a click commits the OTHER option (the model's toggle rule:
/// checked iff current == first option).
fn checkbox_presentation(placed: &PlacedMenu, index: usize, cx: &App) -> impl IntoElement {
    let current = placed.state.read(cx).value.clone();
    let binding = placed.binding.clone();
    let checked = checkbox_checked(&binding.options, Some(&current));

    div()
        .id(SharedString::from(format!("bf-checkbox-{index}")))
        .size_full()
        .flex()
        .items_center()
        .gap_2()
        .px(px(4.0))
        .cursor_pointer()
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .rounded(px(3.0))
                .bg(rgba(WIDGET_FILL_TOKEN))
                .border_1()
                .border_color(rgba(WIDGET_BORDER_TOKEN))
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |el| {
                    el.text_size(px(11.0))
                        .text_color(rgba(WIDGET_ACTIVE_TOKEN))
                        .child("✓")
                }),
        )
        .child(
            div()
                .text_size(px(WIDGET_TEXT_SIZE))
                .text_color(rgba(WIDGET_LABEL_TOKEN))
                .child(binding.param_name.clone()),
        )
        .on_mouse_down(MouseButton::Left, {
            let state = placed.state.clone();
            let coordinator = placed.coordinator.clone();
            let binding = binding.clone();
            move |_ev, window, cx| {
                let current = state.read(cx).value.clone();
                if let Some(i) = checkbox_toggle_index(&binding.options, Some(&current)) {
                    commit_pick(&state, &coordinator, &binding, index, i, window, cx);
                }
            }
        })
}
