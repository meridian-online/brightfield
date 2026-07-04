Field-scan memo: the full Rust UI framework field evaluated as GPUI alternatives, prompted by Hugh's observation that Brightfield's low gpui coupling (~5k LOC / 4% exit price, see `2026-07-05-gpl-precedent-and-dioxus.md`) exists *because* there is no workspace yet — coupling grows with every dock, surface, and binding we build, so the alternatives question had to be answered properly now or never. Six candidates scanned in parallel against weighted axes (keyboard machinery > custom GPU hosting > workspace chrome > text editing > MIT-shippable license/governance > maturity), all claims verified against primary sources (crates.io API, docs.rs, repos, maintainer statements), July 2026. Dioxus was evaluated separately (same memo as above): ruled out.

## Scorecard

| Candidate | Verdict for Brightfield | Wins | Loses |
|---|---|---|---|
| **egui** 0.35 | Closest challenger | Rerun = production GPU data-viz workspace on egui (the architectural cousin); zero-copy `register_native_texture` on shared device; `egui_tiles`/`egui_dock` off the shelf; quarterly MIT releases; AccessKit by default; Rerun's $17M funds the author | No action/keymap system (Rerun's is a flat 2-file command list); TextEdit-grade editing, unsuited to large docs; Linux/CJK IME issues |
| **iced** 0.14 | Close second | Best shipping GPU story (`shader::Primitive` hands you iced's device/queue/render-pass — vello renders in-frame, no blit); `pane_grid` built in; `text_editor`+syntect; verified 483-crate zero-copyleft lockfile; full-time maintainer (Kraken-funded), System76 as second steward; Kraken Desktop + COSMIC DE in production | Weakest keyboard story of the serious candidates (raw `keyboard::listen`, flat KeyBind tables); ~15-month release gaps; mainline a11y absent (fork-only) |
| **xilem/masonry** 0.4 | Worse today; strongest successor candidate | `Widget::paint(…, scene: &mut vello::Scene)` — our Scenes composite natively, both devices and the blit disappear; same Linebender org as our kurbo/peniko/vello stack; Apache-2.0, zero copyleft; AccessKit mandatory in the Widget trait | Self-described alpha; keyboard machinery on the someday-list; no tabs/docks/editor; 3 reverse deps; funding just destabilized (Raph left Google → Canva; sponsorship unconfirmed); paint API currently migrating to an experimental `imaging` abstraction with **no confirmed vello-Scene passthrough** |
| **floem** 0.2 | Worse | MIT modal code editor (Lapce lineage) that GPUI-land lacks license-cleanly; `PaintCx` speaks kurbo/peniko — our mark code retargets nearly 1:1; most architecturally compatible fallback | Bus factor ≈1, ~50 commits/yr, no crates.io release in 20 months, Lapce itself coasting; no keymap/action layer (lives in Lapce the app); no wgpu texture sharing (blit-equivalent) |
| **Slint** 1.17 | Worse | Real release engineering (semver every 6–10 weeks); first-party `Image::try_from(wgpu::Texture)`; LibrePCB 2.0 proof | Compiled `.slint` DSL resists spec-generated, runtime-modal UI; no docking/palette/editor ecosystem; Royalty-Free license is workable but never clean (attribution widget, no-API-exposure clause threatens a future plugin SDK); commercial gravity is embedded |
| **Makepad** 1.0 | Worse | First-party Dock proven outside their studio (Robrix); fully vendored copyleft-free tree | Own shader-DSL graphics stack structurally competes with wgpu/vello (entry = per-frame CPU BGRA upload); zero keyboard machinery; 14-month crates.io stall; flagship pins a contributor's personal fork; a11y pre-alpha |

**Correction applied to agent verdicts:** the iced and egui agents scored GPUI without gpui-component in view ("GPUI's chrome/editor is GPL, effectively zero"). Our actual plan is GPUI + stub patch + gpui-component (Apache-2.0: dock system, tree-sitter/LSP editor rated to 200k-line files, charts). With that correction, GPUI wins the chrome axis (parity) and the editor axis (clearly), and the field's only durable advantage over GPUI is **structural release/governance hygiene** — crates.io cadence, no stub patch, second stewards.

## Decision

**Stay on GPUI. The scan is closed — stop looking; convert vigilance into named fallbacks + an architecture rule.**

Reasoning: the #1-weighted axis (focus-scoped actions/keymaps/palette — the substrate of the VisiData grammar) is a shipped, Zed-proven *system* in GPUI and a DIY project on every single alternative. The #4 axis (production editor) is covered license-cleanly by gpui-component. GPUI's real deficits — GPL stub patch, git-only cadence, single-vendor governance, no public wgpu interop — are permanent but bounded: the stub is 3 shim crates + a CI gate, the pin is ours, gpui is Apache and forkable at any rev (worst-case backstop: vendor the last good rev; a desktop app doesn't need zed's feature velocity). Meanwhile the render core stays framework-free vello, which is exactly the part every attractive alternative (egui texture path, iced Primitive, xilem Scene-append) knows how to host — portability is preserved at the layer that matters.

**Designated fallbacks (no further scanning):**
- **egui** — pragmatic fallback if GPUI's structural risk materializes this era. Rerun is the existence proof for everything hard; migration cost ≈ the coupling-audit price + rebuilding the keyboard layer we'd have built anyway.
- **xilem/masonry** — long-term successor watch. Re-check at 0.5–0.6 on three conditions: `imaging` migration settled with a confirmed vello-Scene passthrough, a keymap/action layer exists, one real production app ships.
- floem — footnote: architecturally easiest port (kurbo/peniko vocabulary, MIT modal editor) but only if it finds a steward.

**Re-open triggers (consolidated with `2026-07-05-gpl-precedent-and-dioxus.md`):** (1) a functional (non-shim) zed crate goes GPL; (2) the zed pin becomes unbuildable/unmaintainable and no clean rev exists; (3) xilem meets its three conditions above; (4) a committed consumer-web sprint (prototype the standalone vello-WASM canvas first — framework-independent either way).

## The architecture rule that caps the exit price

Hugh's observation is the risk: the 4% figure is pre-workspace, and workspace chrome is where frameworks entangle. The mitigation is to make the semantic layer framework-free **by design** — which the VisiData command-log substrate independently requires (every action must be nameable and loggable, so the registry cannot live in framework closures):

1. **Command registry as data** — action names, scope prefixes, bindings, palette entries in a gpui-free module; GPUI's `actions!`/keymap layer *adapts* it, never defines it.
2. **Surface model as a plain state machine** — the surface stack, focus semantics, dive/pop transitions; same pattern as `interaction.rs` today.
3. **Layout/dock state as serializable data** — never framework view-tree state.
4. **Command log** — the spec-edit stream, pure data by construction.
5. **Views/elements stay translation shims** — framework events in, registry commands out; rendering = compose vello scenes + blit.

Held to this, the workspace era adds mostly *thin* coupled code and the exit price grows sublinearly — the insurance and the product feature (replayable command log) are the same work. Review checkpoint for every UI PR: could this file's logic run headless? If not, it had better be a shim.

→ Applies immediately to the framed-window card (WorkspaceView, LegendElement) and the keyboard-grammar card (`2026-07-04-visidata-keyboard-grammar.md`). No new card from this memo; it closes the framework question.

## Same-day addendum: the Linebender-alignment question

Hugh asked whether the Linebender stack has better *future* alignment for Brightfield. Assessment: **yes directionally, but it is two separate bets, and only one is ready.**

**Bet A — the rendering stack (vello/kurbo/peniko/skrifa): already made, and it is where the alignment already pays.** brightfield-render (6.8k LOC, the differentiating core) is pure Linebender; every attractive future shell paints with or hosts it (masonry natively, Blitz natively, floem via kurbo/peniko vocabulary, egui/iced via texture); and the consumer-web future is Linebender-shaped regardless of shell (vello WASM/WebGPU canvas). Linebender is an infrastructure community with multiple corporate consumers (Google Fonts era, Canva now employing Raph on rendering) — for a rendering-centric product, a more natural long-term home than an app company's framework (Zed).

**Bet B — the widget framework (masonry/xilem): not ready to carry the workspace era.** Alpha, keyboard machinery on the someday-list, no chrome/editor, funding freshly wobbled, paint API mid-migration to the experimental `imaging` abstraction with no confirmed vello-Scene passthrough. The successor-watch triggers stand (0.5–0.6 + vello passthrough + keymap layer + one production app).

**The uncomfortable local finding: we are currently drifting AWAY from the stack we claim alignment with.** Verified in-lock: vello 0.4.1 / kurbo 0.11.3 / peniko 0.3.2 / wgpu 23.0.1 — while the ecosystem line is vello ~0.8 / kurbo 0.13 / peniko 0.6 / wgpu 28 (four vello majors, five wgpu majors behind). If future alignment is the strategy, the first concrete move is **a Linebender-bump chore card**, not a shell change. The blit isolation makes it self-contained: `vello_renderer.rs` owns its own wgpu device, gpui renders via Metal (no shared wgpu), so the bump touches only render/ui crates. Expect mechanical-but-broad churn in scene-building code (kurbo/peniko type changes across mark.rs); note the lock already carries a second peniko (0.4.1) — identify its consumer during the bump. Benefits: vello perf/quality improvements, staying compatible with where masonry/Blitz/floem live, shrinking the eventual migration delta, keeping a vello-WASM consumer-mode prototype current.

**The strategy in one line:** GPUI shell for the workspace era (it ships the systems the era needs) + fresh Linebender core + framework-free semantic layer — so that if masonry matures per the triggers, the eventual shell migration is the cheap kind and its payoff (native `Scene::append`, delete the second device and the blit) is maximal.

→ Card candidate: "Linebender stack bump" (chore; headlessly verifiable — the PNG conformance suite is the regression harness).
