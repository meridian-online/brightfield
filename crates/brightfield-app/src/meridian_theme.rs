//! Meridian design-system installation (design phase 4 PR A): the bundled
//! Inter / JetBrains Mono faces and the emitted gpui-component theme pair.
//!
//! Both run inside `app.run`, bracketing `gpui_component::init` — fonts
//! before any window opens (so the theme's font families resolve to the
//! bundled bytes, never a system lookalike), the theme immediately after
//! init has seeded the stock global. Chrome only: the vello chart canvas
//! reads none of this, so the example PNGs stay byte-identical (the
//! design-adoption acceptance gate; chart ink is PR B's).

use std::borrow::Cow;
use std::rc::Rc;

use gpui::App;
use gpui_component::{Theme, ThemeConfig};
use meridian_design::emit::{theme_config, ThemeMode as MeridianMode};

/// Register the bundled Inter + JetBrains Mono faces with the text system.
///
/// Must run BEFORE any window opens. Registration failure is non-fatal —
/// the theme's families then fall back to the platform's font resolution,
/// which is degraded chrome, not a broken app.
pub fn register_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = meridian_design::fonts::ALL
        .iter()
        .map(|bytes| Cow::Borrowed(*bytes))
        .collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!(
            "meridian fonts: failed to register the bundled {} / {} faces ({e}); \
             falling back to system font resolution",
            meridian_design::fonts::SANS,
            meridian_design::fonts::MONO,
        );
    }
}

/// Parse the emitted Meridian `ThemeConfig` pair and install it on the
/// gpui-component theme global, with the LIGHT theme active.
///
/// Must run after `gpui_component::init` (which seeds the global with the
/// stock theme). Both configs are wired into the theme's light/dark pair
/// storage, but system-appearance sync is deliberately NOT enabled: the
/// vello chart canvas is light-only until PR B lands dark chart ink, and
/// dark chrome around white charts would read as broken. A later PR flips
/// the mode / adds the sync without re-plumbing.
///
/// The `expect`s are safe-by-test: `meridian_theme_schema` pins both
/// emitted JSONs against this gpui-component pin's `ThemeConfig` schema.
pub fn install_theme(cx: &mut App) {
    let light: ThemeConfig = serde_json::from_str(&theme_config(MeridianMode::Light))
        .expect("meridian-design light ThemeConfig drifted from the gpui-component schema pin");
    let dark: ThemeConfig = serde_json::from_str(&theme_config(MeridianMode::Dark))
        .expect("meridian-design dark ThemeConfig drifted from the gpui-component schema pin");

    let theme = Theme::global_mut(cx);
    theme.light_theme = Rc::new(light);
    theme.dark_theme = Rc::new(dark);
    // Re-apply the (now Meridian) light config onto the live theme. No
    // window exists yet, so no refresh handle to pass.
    Theme::change(gpui_component::ThemeMode::Light, None, cx);
}

/// Schema-drift regression tests: the design repo's emitter is pinned
/// against THIS gpui-component pin's `ThemeConfig` schema. `ThemeConfig`
/// is `#[serde(default)]` and tolerant of unknown keys, so a renamed field
/// would parse "successfully" into `None` and silently fall back to the
/// stock colour — hence the non-default spot assertions alongside the
/// plain parse success.
#[cfg(test)]
mod meridian_theme_schema {
    use gpui_component::{ThemeConfig, ThemeMode};
    use meridian_design::emit::{theme_config, ThemeMode as MeridianMode};

    fn parse(mode: MeridianMode) -> ThemeConfig {
        let json = theme_config(mode);
        serde_json::from_str::<ThemeConfig>(&json)
            .unwrap_or_else(|e| panic!("emitted ThemeConfig no longer parses: {e}\n{json}"))
    }

    #[test]
    fn light_config_parses_with_non_default_fields() {
        let light = parse(MeridianMode::Light);
        assert_eq!(light.name.as_ref(), "Meridian Light");
        assert_eq!(light.mode, ThemeMode::Light);
        // Fonts and metrics land as typed options, not serde defaults.
        assert_eq!(light.font_family.as_deref(), Some("Inter"));
        assert_eq!(light.font_size, Some(12.0));
        assert_eq!(light.mono_font_family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(light.mono_font_size, Some(12.0));
        assert_eq!(light.radius, Some(6));
        assert_eq!(light.radius_lg, Some(8));
        // Spot colour fields across the families the shell renders: a key
        // rename upstream would silently None these (serde ignores unknown
        // keys), reverting that family to the stock blue.
        assert!(light.colors.background.is_some(), "background missing");
        assert!(light.colors.primary.is_some(), "primary.background missing");
        assert!(light.colors.ring.is_some(), "ring missing");
        assert!(light.colors.sidebar.is_some(), "sidebar.background missing");
        assert!(light.colors.tab_active.is_some(), "tab.active.background missing");
        assert!(light.colors.title_bar.is_some(), "title_bar.background missing");
        // Maritime primary, verbatim (web decision 0010: hsl(205, 35%, 45%)).
        assert_eq!(light.colors.primary.as_deref(), Some("#4b7a9b"));
    }

    #[test]
    fn dark_config_parses_with_non_default_fields() {
        let dark = parse(MeridianMode::Dark);
        assert_eq!(dark.name.as_ref(), "Meridian Dark");
        assert_eq!(dark.mode, ThemeMode::Dark);
        assert_eq!(dark.font_family.as_deref(), Some("Inter"));
        assert_eq!(dark.mono_font_family.as_deref(), Some("JetBrains Mono"));
        assert!(dark.colors.background.is_some(), "background missing");
        assert!(dark.colors.primary.is_some(), "primary.background missing");
        assert!(dark.colors.ring.is_some(), "ring missing");
    }
}
