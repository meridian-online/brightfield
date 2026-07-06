//! Boot-mode seam (card 0017, aws_ac01) — framework-free.
//!
//! The ONE decision separating the headless PNG dump from the windowed
//! authoring workspace, lifted out of `main` so it is headlessly testable
//! and structurally reviewable: `main` matches on [`BootMode`] and RETURNS
//! from the dump arm before any workspace/dock construction is reachable.
//! The workspace shell (DockArea, panels, editor — `shell.rs`) is only
//! called from the [`BootMode::Window`] arm, so no shell state can ever
//! move a pixel in a dumped PNG (the byte-identity halt gate's structural
//! guarantee). No gpui import may enter this file (semantic-layer rule).

/// How this process runs, decided once at startup from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootMode {
    /// `BRIGHTFIELD_DUMP_PNG=<path>`: composite the dashboard, write the
    /// PNG, and return — before the workspace shell exists.
    HeadlessDump(String),
    /// No dump variable: open the native window hosting the workspace.
    Window,
}

/// Decide the boot mode from the `BRIGHTFIELD_DUMP_PNG` variable's value
/// (`None` = unset). Pure so the seam is pinned by a unit test.
#[must_use]
pub fn boot_mode(dump_png: Option<String>) -> BootMode {
    match dump_png {
        Some(path) => BootMode::HeadlessDump(path),
        None => BootMode::Window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// aws_ac01 (seam): the dump variable short-circuits to the headless
    /// arm — the only arm `main` reaches before `return` — and its absence
    /// is the only route to the workspace. Presentation/dock state appears
    /// nowhere in the decision, so shell work cannot affect PNG output.
    #[test]
    fn aws_ac01_dump_mode_short_circuits_before_workspace() {
        assert_eq!(
            boot_mode(Some("/tmp/out.png".to_string())),
            BootMode::HeadlessDump("/tmp/out.png".to_string())
        );
        assert_eq!(boot_mode(None), BootMode::Window);
    }
}
