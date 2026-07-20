//! The one `egui_tiles::Behavior` in the product.
//!
//! `egui_tiles` asks a `Behavior` two things about a pane: what to call it,
//! and how to draw it. [`PaneChrome`] answers both from the same place — the
//! pane's [`Subject`](crate::Subject) — so a tab title and a header band
//! cannot say different things about the same pane, which is precisely what
//! happened in the shell this replaces.
//!
//! # What the no-document-handle design buys here, and what it does not
//!
//! `tab_title_for_pane` is called for *every* tab in a strip, including the
//! ones not being drawn, and its title comes from the document. It is worth
//! being exact about the borrow that permits that, because the loose version
//! of the argument — that four tabs would need four simultaneous handles to
//! one document — is not true, and was written down here before anyone
//! checked it. `egui_tiles` 0.16 draws a tab bar *before* the active child's
//! `pane_ui`, never nested inside it, so the calls are sequential.
//! Instrumenting a real frame gives `title, title, pane-in, pane-out,
//! pane-in, pane-out`: no title is ever asked for while a pane is mid-draw,
//! so an `Rc<RefCell<_>>` would not panic here, and "it compiles" would prove
//! only that four sequential `&mut self` calls compile, which nearly any
//! ownership shape manages.
//!
//! What the shape does buy is a level down, and is about aliasing *within*
//! one call rather than across several. [`PaneChrome`] holds the document and
//! the item map as two fields, and `pane_ui` destructures `&mut self` so the
//! borrow checker sees them as disjoint: an item is borrowed mutably out of
//! `items` and handed `&mut **doc` in the same expression. Were the document
//! reached *through* the item, that line would be `item.ui(&mut item.doc, …)`
//! — one `&mut` to the item and a second reaching through it — which does not
//! compile, and the usual escape is an `Rc<RefCell<_>>` whose discipline is
//! then checked at runtime instead of by the compiler. That is the whole
//! claim: smaller than the one it replaces, and the one the code actually
//! supports. Delete the destructure in `pane_ui` and the borrow error is
//! immediate.

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
        //
        // Known imprecision, written down before anything is built on it:
        // `any_pressed` is global and `outer` is this pane's rect as it was
        // before the body drew, so a press inside a floating `egui::Area` — a
        // popup, a context menu, a tooltip with a button in it — that happens
        // to overlap this pane also raises `Request::Focus` for the pane
        // underneath. The effect is that window chrome follows the pane the
        // popup covers rather than the pane that opened it. Cosmetic while
        // the views have no popups; the fix, when one of them does, is to ask
        // egui which layer the press landed in rather than only where it was.

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
