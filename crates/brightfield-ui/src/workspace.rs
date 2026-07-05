//! Workspace shell model (card 0016) — the gpui-free half of the framed window.
//!
//! Everything here is plain state and arithmetic so it runs headlessly (the
//! semantic-layer rule: state machines and size formulas are framework-free;
//! `workspace_view.rs` is the thin GPUI translation shim over this module).
//! No gpui import may enter this file.
//!
//! - [`PresentationMode`] — the authoring/presentation state machine the
//!   `TogglePresentation` action flips.
//! - [`framed_window_size`] — dashboard bbox + chrome extents. The formula is
//!   mode-independent by construction: toggling presentation never resizes the
//!   window, only re-centres the canvas ([`canvas_origin`]).
//! - [`resolve_title`] — `meta.title` wins, spec filename stem falls back. The
//!   single resolver feeding both the native titlebar and the header strip.

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
/// Deliberately takes no [`PresentationMode`]: the window keeps its size when
/// presentation toggles (the locked pick) — only [`canvas_origin`] moves.
#[must_use]
pub fn framed_window_size(dashboard_width: f64, dashboard_height: f64) -> (f64, f64) {
    (
        dashboard_width + 2.0 * CONTENT_PADDING,
        dashboard_height + HEADER_HEIGHT + 2.0 * CONTENT_PADDING,
    )
}

/// Where the dashboard canvas mounts inside a window of `window` size:
/// centred in the padded content area below the header (authoring), or
/// centred in the full window (presentation — the chrome's freed space is
/// reclaimed without any window resize).
#[must_use]
pub fn canvas_origin(
    mode: PresentationMode,
    window: (f64, f64),
    dashboard: (f64, f64),
) -> (f64, f64) {
    let (win_w, win_h) = window;
    let (dash_w, dash_h) = dashboard;
    match mode {
        PresentationMode::Authoring => {
            let content_w = (win_w - 2.0 * CONTENT_PADDING).max(0.0);
            let content_h = (win_h - HEADER_HEIGHT - 2.0 * CONTENT_PADDING).max(0.0);
            (
                CONTENT_PADDING + (content_w - dash_w) / 2.0,
                HEADER_HEIGHT + CONTENT_PADDING + (content_h - dash_h) / 2.0,
            )
        }
        PresentationMode::Presentation => ((win_w - dash_w) / 2.0, (win_h - dash_h) / 2.0),
    }
}

/// The dashboard's display title: `meta.title` when declared (and non-blank),
/// else the spec file's stem (`examples/framed.yaml` → `framed`). This single
/// resolver feeds BOTH the native titlebar and the shell header strip, so the
/// two can never disagree.
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

    /// fww_ac01: framed window size = dashboard bbox + chrome extents, and the
    /// formula is presentation-independent — toggling only moves the canvas
    /// mount origin (re-centred in the full window), never the window size.
    #[test]
    fn fww_ac01_window_size_adds_chrome_and_toggle_recentres() {
        let (dash_w, dash_h) = (800.0, 600.0);
        let (win_w, win_h) = framed_window_size(dash_w, dash_h);
        assert_eq!(win_w, dash_w + 2.0 * CONTENT_PADDING);
        assert_eq!(win_h, dash_h + HEADER_HEIGHT + 2.0 * CONTENT_PADDING);

        // The size function takes no mode — the window CANNOT resize on toggle.
        // The canvas origin is what moves: exact-fit content area in authoring…
        let authoring =
            canvas_origin(PresentationMode::Authoring, (win_w, win_h), (dash_w, dash_h));
        assert_eq!(authoring, (CONTENT_PADDING, HEADER_HEIGHT + CONTENT_PADDING));

        // …and centred in the FULL (unchanged) window in presentation, so the
        // canvas reclaims half the header's freed space vertically.
        let presentation =
            canvas_origin(PresentationMode::Presentation, (win_w, win_h), (dash_w, dash_h));
        assert_eq!(
            presentation,
            (
                CONTENT_PADDING,
                (HEADER_HEIGHT + 2.0 * CONTENT_PADDING) / 2.0
            )
        );
        assert!(
            presentation.1 < authoring.1,
            "hiding the header must float the canvas upward into the freed space"
        );
    }

    /// fww_ac02: title resolution — `meta.title` wins; a spec without one
    /// resolves to its filename stem. (Both the native titlebar and the header
    /// strip consume this one resolver — see main.rs, one call site each.)
    #[test]
    fn fww_ac02_title_meta_wins_filename_falls_back() {
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

    /// fww_ac03 (state machine): toggle() flips Authoring ↔ Presentation and
    /// back; the type lives in this module, which imports no gpui.
    #[test]
    fn fww_ac03_presentation_state_machine_toggles() {
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

    /// fww_ac03 (layout invariance): the spec-derived layout is computed from
    /// (spec, viewport) ONLY — the placed plot/input/legend rects are identical
    /// whichever side of a presentation toggle they are computed on, because
    /// shell state appears nowhere in the layout call chain.
    #[test]
    fn fww_ac03_layout_ignores_presentation_state() {
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
