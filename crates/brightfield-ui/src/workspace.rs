//! Workspace shell model — the gpui-free half of the window shell.
//!
//! Everything here is plain state and arithmetic so it runs headlessly (the
//! semantic-layer rule: state machines and size formulas are framework-free;
//! the GPUI views are thin translation shims over this module). No gpui
//! import may enter this file.
//!
//! - [`PresentationMode`] — the authoring/presentation state machine the
//!   `TogglePresentation` action flips (hosted by the canvas panel).
//! - [`framed_window_size`] — dashboard bbox + chrome extents: the INITIAL
//!   window-size formula. The "window never resizes on toggle" invariant
//!   this formula once anchored is SUPERSEDED by the DockArea shell:
//!   the DockArea owns layout once the window is open, and the app's
//!   `shell_model::initial_window_size` adds the default dock widths on top
//!   of this formula. The oracle is revised to match.
//! - [`resolve_title`] — `meta.title` wins, spec filename stem falls back.
//!   The single resolver feeding both the native titlebar and the canvas
//!   panel's tab title.

use std::path::Path;

/// Header strip height in logical pixels (the title bar drawn by the shell,
/// below the native titlebar).
pub const HEADER_HEIGHT: f64 = 36.0;

/// Padding around the canvas inside the content area, in logical pixels.
pub const CONTENT_PADDING: f64 = 16.0;

/// The workspace surface state: authoring chrome shown, or presentation
/// (chrome hidden, canvas only — exactly what a consumer would see).
///
/// A plain two-state machine so the shell's one bit of mode lives in
/// headlessly-testable code; the GPUI view reads it, never owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PresentationMode {
    /// Shell chrome visible: header strip + padded content area.
    #[default]
    Authoring,
    /// Chrome hidden: only the spec-derived canvas, centred in the window.
    Presentation,
}

impl PresentationMode {
    /// The other mode (pure form of [`toggle`](Self::toggle)).
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Authoring => Self::Presentation,
            Self::Presentation => Self::Authoring,
        }
    }

    /// Flip Authoring ↔ Presentation in place.
    pub fn toggle(&mut self) {
        *self = self.toggled();
    }

    /// Whether the shell chrome (header strip, content padding) is shown.
    #[must_use]
    pub fn chrome_visible(self) -> bool {
        matches!(self, Self::Authoring)
    }
}

/// Framed window content size for a dashboard bounding box: the canvas plus
/// the chrome extents (header strip above, content padding on every side).
///
/// INITIAL size only: the docked workspace feeds this through
/// the app's `shell_model::initial_window_size` (adding the default dock
/// widths) to choose the first-boot window size — once open, the DockArea
/// owns layout and the user owns the window size. The "toggling
/// presentation never resizes the window" invariant this formula once
/// anchored is superseded. The formula
/// still takes no [`PresentationMode`], which now guarantees only that the
/// initial size is mode-independent.
#[must_use]
pub fn framed_window_size(dashboard_width: f64, dashboard_height: f64) -> (f64, f64) {
    (
        dashboard_width + 2.0 * CONTENT_PADDING,
        dashboard_height + HEADER_HEIGHT + 2.0 * CONTENT_PADDING,
    )
}

/// The dashboard's display title: `meta.title` when declared (and non-blank),
/// else the spec file's stem (`examples/framed.yaml` → `framed`). This single
/// resolver feeds BOTH the native titlebar and the canvas panel's tab title,
/// so the two can never disagree.
#[must_use]
pub fn resolve_title(meta_title: Option<&str>, spec_path: &str) -> String {
    if let Some(title) = meta_title {
        if !title.trim().is_empty() {
            return title.to_string();
        }
    }
    Path::new(spec_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Brightfield".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Revised for the DockArea shell — the ONE sanctioned oracle revision:
    /// framed window size = dashboard bbox +
    /// chrome extents, as the INITIAL window-size formula. The formula
    /// takes no `PresentationMode`, which now pins only that the
    /// first-boot size is mode-independent — the "window never
    /// resizes on toggle" invariant is superseded (the DockArea owns
    /// layout once the window is open, and the app's dock-layout tests pin
    /// the dock widths added on top). The historical `canvas_origin`
    /// re-centre oracle that mirrored `WorkspaceView`'s flex layout is
    /// deleted with the invariant it asserted; the live counterpart is
    /// the CanvasPanel's flex centring inside the DockArea
    /// (an eyeball check).
    #[test]
    fn initial_window_size_adds_chrome_extents() {
        let (dash_w, dash_h) = (800.0, 600.0);
        let (win_w, win_h) = framed_window_size(dash_w, dash_h);
        assert_eq!(win_w, dash_w + 2.0 * CONTENT_PADDING);
        assert_eq!(win_h, dash_h + HEADER_HEIGHT + 2.0 * CONTENT_PADDING);
    }

    /// Title resolution — `meta.title` wins; a spec without one
    /// resolves to its filename stem. (Both the native titlebar and the
    /// canvas panel's tab title consume this one resolver — see main.rs,
    /// one call site.)
    #[test]
    fn title_meta_wins_filename_falls_back() {
        assert_eq!(
            resolve_title(Some("Sales Overview"), "examples/dashboard.yaml"),
            "Sales Overview"
        );
        assert_eq!(
            resolve_title(None, "examples/legend-standalone.yaml"),
            "legend-standalone"
        );
        // A blank title is treated as absent rather than shown as nothing.
        assert_eq!(resolve_title(Some("   "), "examples/framed.yaml"), "framed");
    }

    /// State machine: toggle() flips Authoring ↔ Presentation and
    /// back; the type lives in this module, which imports no gpui.
    #[test]
    fn presentation_state_machine_toggles() {
        let mut mode = PresentationMode::default();
        assert_eq!(mode, PresentationMode::Authoring);
        assert!(mode.chrome_visible());

        mode.toggle();
        assert_eq!(mode, PresentationMode::Presentation);
        assert!(!mode.chrome_visible());

        mode.toggle();
        assert_eq!(mode, PresentationMode::Authoring, "toggle is symmetric");
        assert_eq!(mode.toggled().toggled(), mode, "pure form round-trips");
    }

    /// Layout invariance: the spec-derived layout is computed from
    /// (spec, viewport) ONLY — the placed plot/input/legend rects are identical
    /// whichever side of a presentation toggle they are computed on, because
    /// shell state appears nowhere in the layout call chain.
    #[test]
    fn layout_ignores_presentation_state() {
        use brightfield_spec::layout::{
            placed_input_nodes, placed_legend_nodes, placed_plots, Rect,
        };
        use brightfield_spec::{parse_spec, Format};

        let yaml = r#"
data:
  t:
    - { x: 1, y: 2, g: a }
    - { x: 3, y: 4, g: b }
hconcat:
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      fill: g
    name: scatter
  - legend: color
    for: scatter
"#;
        let spec = parse_spec(yaml, Format::Yaml).expect("spec parses").spec;
        let viewport = Rect::new(0.0, 0.0, 0.0, 0.0);

        let rects = |spec: &brightfield_spec::ast::Spec| -> Vec<(f64, f64, f64, f64)> {
            placed_plots(spec, viewport)
                .iter()
                .map(|p| (p.rect.x, p.rect.y, p.rect.width, p.rect.height))
                .chain(
                    placed_input_nodes(spec, viewport)
                        .iter()
                        .map(|(r, _)| (r.x, r.y, r.width, r.height)),
                )
                .chain(
                    placed_legend_nodes(spec, viewport)
                        .iter()
                        .map(|(r, _)| (r.x, r.y, r.width, r.height)),
                )
                .collect()
        };

        let mut mode = PresentationMode::Authoring;
        let before = rects(&spec);
        mode.toggle();
        let after = rects(&spec);
        assert_eq!(
            before, after,
            "same inputs, same outputs: layout takes (spec, viewport), never {mode:?}"
        );
        assert!(!before.is_empty(), "the probe spec places at least one rect");
    }
}
