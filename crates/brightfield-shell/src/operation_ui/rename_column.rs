//! `rename-column` in the window: not built yet, so its slot is empty.

use super::OperationUi;

/// `rename-column`'s slot: `None` until the operation is built.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    None
}
