//! The one `egui_tiles::Behavior` in the product.
//!
//! `egui_tiles` asks a `Behavior` two things about a pane: what to call it,
//! and how to draw it. [`PaneChrome`] answers both from the same place — the
//! pane's [`Subject`](crate::Subject) — so a tab title and a header band
//! cannot say different things about the same pane, which is precisely what
//! happened in the shell this replaces.
//!
//! # This is the case the no-document-handle design exists to permit
//!
//! `tab_title_for_pane` is called for *every* tab in a strip, including the
//! ones not being drawn. Its title comes from the document. If an item held
//! its own handle to the document, answering that question for four tabs
//! would need four simultaneous handles to one document, two of which mutate
//! on click — four `&mut`, which is not a thing, or an `Rc<RefCell<_>>` whose
//! `borrow_mut` panics the first time a title is asked for while a pane is
//! mid-draw.
//!
//! Because an [`Item`] holds nothing, [`PaneChrome`] holds `&mut D` and
//! `&mut ItemMap<D>` as two disjoint fields, and both `tab_title_for_pane`
//! and `pane_ui` reach the document through them. Exactly one `&mut D` exists
//! at a time and it lives for one call. That this compiles at all is the
//! proof the shape works; nothing else about it needs arguing.

use std::collections::HashSet;

use egui_tiles::{SimplificationOptions, TileId, UiResponse};

use crate::chrome;
use crate::item::{ItemCtx, ItemMap, PaneKey, Request};
use crate::Mode;

/// Draws every pane in one view's tree, and names every tab in it.
///
/// Borrowed rather than owning, and built fresh each frame: it holds the
/// view's document and the view's live items for exactly as long as
/// `Tree::ui` runs.
pub struct PaneChrome<'a, D: ?Sized> {
    /// The view's document. The single `&mut D`.
    doc: &'a mut D,
    /// The view's live items, addressed by pane.
    items: &'a mut ItemMap<D>,
    /// Light or dark.
    mode: Mode,
    /// The focused pane, if any — the shell's, not this type's, decision.
    focused: Option<PaneKey>,
    /// Tiles whose parent is a tab container, so their header band is
    /// suppressed. Computed by
    /// [`Workspace::tabbed_tiles`](crate::Workspace::tabbed_tiles) before
    /// `Tree::ui` took its borrow of the tree, because a `Behavior` cannot
    /// hold a reference to the tree it is being run against.
    tabbed: &'a HashSet<TileId>,
    /// Where deferred work goes: verbs the user activated in chrome, and
    /// focus moves. Drained by the shell after `Tree::ui` returns, for the
    /// reason [`Request`] documents — acting on one now would mean
    /// re-entering the tile tree from inside its own draw.
    requests: &'a mut Vec<Request>,
}

impl<'a, D: ?Sized> PaneChrome<'a, D> {
    /// Build the behaviour for one view's frame.
    pub fn new(
        doc: &'a mut D,
        items: &'a mut ItemMap<D>,
        mode: Mode,
        focused: Option<PaneKey>,
        tabbed: &'a HashSet<TileId>,
        requests: &'a mut Vec<Request>,
    ) -> Self {
        Self {
            doc,
            items,
            mode,
            focused,
            tabbed,
            requests,
        }
    }
}

impl<D: ?Sized> egui_tiles::Behavior<PaneKey> for PaneChrome<'_, D> {
    /// The tab title, read from a pane that is **not** drawing.
    ///
    /// A pane whose item this build cannot construct still gets a title —
    /// its key — rather than an empty tab. A nameless tab beside named ones
    /// reads as a rendering bug; the key names the actual cause.
    fn tab_title_for_pane(&mut self, pane: &PaneKey) -> egui::WidgetText {
        match self.items.get(pane) {
            Some(item) => item.subject(self.doc).title.into(),
            None => pane.to_string().into(),
        }
    }

    /// Draw one pane: its chrome from its subject, then its body.
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile: TileId, pane: &mut PaneKey) -> UiResponse {
        // Destructured so the borrow checker sees `doc`, `items` and
        // `requests` as three disjoint fields rather than one `&mut self`.
        // This is the borrow the whole contract is arranged around.
        let Self {
            doc,
            items,
            mode,
            focused,
            tabbed,
            requests,
        } = self;
        let mode = *mode;
        let key = *pane;

        let Some(item) = items.get_mut(&key) else {
            chrome::orphan_pane(ui, key, mode);
            return UiResponse::None;
        };

        let outer = ui.max_rect();
        let subject = item.subject(&**doc);
        let mut body = chrome::pane_frame(ui, &subject, !tabbed.contains(&tile), mode);

        if let Some(empty) = &subject.empty_state {
            // The empty state is drawn *instead of* the item, which is what
            // makes it impossible to forget: it is not a branch a pane author
            // has to remember to write inside their own draw.
            if let Some(verb) = chrome::empty_state(&mut body, empty, mode).activated {
                requests.push(Request::Verb(verb));
            }
        } else {
            let mut cx = ItemCtx::new(mode, key, tile, *focused == Some(key), requests);
            item.ui(doc, &mut body, &mut cx);
        }

        // Focus follows a press anywhere in the pane, observed rather than
        // consumed: `Ui::interact` over the whole pane would sit in front of
        // every widget the item just drew and swallow their clicks, so this
        // reads the pointer state the item has already had its chance at.
        if ui.rect_contains_pointer(outer)
            && ui.input(|i| i.pointer.any_pressed())
            && *focused != Some(key)
        {
            requests.push(Request::Focus(key));
        }

        UiResponse::None
    }

    /// Simplification stays **off**, and that is load-bearing twice.
    ///
    /// `egui_tiles`' default options prune and collapse containers during
    /// layout. That would rewrite the tree from inside the draw, which the
    /// dirty tracker would correctly read as a layout change and write to
    /// disk — a file rewritten on the first frame of every boot. It would
    /// also silently dissolve a view's declared arrangement, and the registry
    /// is meant to be the single declaration of that.
    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions::OFF
    }
}
