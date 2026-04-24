# Spec Review

**Date:** 2026-04-24
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-24-gpu-mark-rendering/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 4 |
| 2 — Assumption & failure | content signals (GPU/wgpu device, cross-crate boundary) | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] AC-01 describes ChartState but spec splits responsibility ambiguously with existing ChartElement
**Category:** missing-requirement
**Pass:** 1
**Description:** AC-01 introduces ChartState as a new struct holding scene, InteractionState, layout dimensions, and a shared VelloRenderer reference. The existing ChartElement (chart_element.rs) already holds scene, InteractionState, width, and height. The implementation note says "evolve it rather than replacing," but the spec never states whether ChartElement's fields migrate to ChartState, whether ChartElement retains its own copies, or how ownership is divided. The spec describes two structs (ChartState and ChartElement) that appear to hold overlapping state.
**Evidence:** AC-01 lists "vello::Scene, InteractionState, layout dimensions (width, height), and a shared reference to the VelloRenderer" as ChartState fields. The existing ChartElement struct (chart_element.rs:16-25) holds scene, interaction, width, height. AC-04 says ChartElement "calls VelloRenderer::render_to_pixels() with the current scene" — implying ChartElement accesses the scene, but should it come from ChartState or its own field?
**Recommendation:** Add a constraint or implementation note clarifying that ChartElement becomes a lightweight rendering shell (no owned state) that borrows from ChartState during paint. State the data flow: ChartView owns Entity<ChartState>, ChartElement receives a reference/clone of the scene and renderer for the duration of one paint cycle.

### [LOW] AC-10 verification is vague about which InteractorKind variants should flip
**Category:** test-gap
**Pass:** 1
**Description:** AC-10 says "InteractorKind variants that depend on the GPUI Element impl are updated to Implemented" and "At minimum: Highlight (already Implemented from card 0010 — verify preserved)." Highlight is already Implemented in vocab.rs. The spec does not name any variant that this card actually flips from non-Implemented to Implemented. If no vocab status changes are needed, AC-10 is a no-op assertion. If some are needed, they should be listed.
**Evidence:** vocab.rs shows Highlight is already Implemented (line 206). The interval-selection interactors (IntervalX, IntervalY, IntervalXY) are Unimplemented, but none of the 10 ACs implement interval selection — they only implement brushing and hovering at the InteractionState level, not wired to the Mosaic interactor spec pipeline.
**Recommendation:** Either (a) remove AC-10 if no vocab status changes are expected from this card, or (b) explicitly list which variants flip and why — e.g., if the GPUI Element impl enables Toggle or Region, name them.

### [MEDIUM] AC-09 gate verification is minimal — "cargo test --workspace — all green" does not cover GPU-dependent tests
**Category:** test-gap
**Pass:** 1
**Description:** AC-09 is a gate AC requiring all existing tests to pass. Its verification is "cargo test --workspace — all green." However, AC-02 and AC-04 introduce tests that require a wgpu device (GPU). On CI or headless environments, wgpu device creation may fail. The spec does not address whether GPU-dependent tests are gated behind a feature flag or cfg attribute.
**Evidence:** AC-02 verification: "create a VelloRenderer, render a simple Scene (one circle) to pixels." This requires wgpu::Instance::new() → request_adapter() → request_device(). In headless CI, request_adapter() returns None on many runners.
**Recommendation:** Add a constraint or implementation note: GPU-requiring tests use `#[cfg(feature = "gpu-tests")]` or `#[ignore]` with a comment, so `cargo test --workspace` (the gate) passes on headless CI. Document the expected test invocation for GPU tests separately.

### [LOW] Content signal: hitbox registration location inconsistency
**Category:** constraint-conflict
**Pass:** 1
**Description:** AC-04 says "prepaint() registers a hitbox covering the element bounds." The interview (Q4) says "Register one hitbox covering the full chart element during paint()." These are different GPUI Element lifecycle phases. The distinction matters because hitbox registration in prepaint vs paint affects event routing timing.
**Evidence:** AC-04: "prepaint() registers a hitbox"; Interview Q4: "during paint()."
**Recommendation:** Clarify in AC-04 which phase registers the hitbox. GPUI's Element trait typically expects hitbox registration in prepaint. If prepaint is correct, no change needed beyond fixing the interview record for consistency.

### [MEDIUM] Assumption: synchronous render_to_pixels in paint() completes within frame budget
**Category:** assumption
**Pass:** 2
**Description:** The spec and interview assert that synchronous Vello rendering in paint() completes in <5ms on Apple Silicon. This assumption is untested at spec time and has no fallback defined. If a complex scene exceeds 16ms, the entire GPUI render loop stalls — there is no frame-skip or degradation path.
**Evidence:** Interview Q2: "On Apple Silicon unified memory, this completes in <5ms for typical charts, well within the 16ms frame budget." Constraint: "Vello rendering is synchronous in paint() — no background threading for v2." No AC measures or asserts frame timing.
**Recommendation:** This is acceptable for v2 scope (the spec acknowledges it explicitly), but add an implementation note or exit condition: "If profiling reveals paint() exceeds 10ms for test scenes, file a card for async render extraction before shipping." This makes the assumption auditable without over-engineering v2.

### [MEDIUM] Assumption: wgpu device creation always succeeds on target platform
**Category:** assumption
**Pass:** 2
**Description:** AC-02 specifies VelloRenderer::new() creates a standalone wgpu device. The spec constrains to Apple Silicon (macOS Metal via wgpu). However, wgpu device creation is async and fallible — request_adapter() can return None, request_device() can fail. The spec does not address the error path for VelloRenderer construction.
**Evidence:** AC-02: "VelloRenderer::new() creates a standalone wgpu device." Implementation note: "wgpu device creation: wgpu::Instance::new() → request_adapter() → request_device() — async but can block_on for v2." No error handling AC.
**Recommendation:** Specify that VelloRenderer::new() returns Result<Self, VelloRendererError> (or panics with a clear message). Since the app cannot function without a GPU device, a panic with a diagnostic message is acceptable for v2, but the spec should state this explicitly rather than leaving the error path undefined.

### [LOW] NavigationState missing from AC-01 ChartState fields
**Category:** missing-requirement
**Pass:** 2
**Description:** The interview (Q1) says ChartState holds "InteractionState, NavigationState, Transition state, and layout dimensions." AC-01 lists "vello::Scene, InteractionState, layout dimensions (width, height), and a shared reference to the VelloRenderer" but omits NavigationState and Transition. If these are deferred to a later card, that should be stated.
**Evidence:** Interview Q1 vs AC-01 field list. NavigationState already exists in interaction.rs and is functional.
**Recommendation:** Either add NavigationState and Transition to the AC-01 field list, or add an implementation note: "NavigationState and Transition integration into ChartState is deferred — they remain separate until the event-routing card wires them."

---

## Honest Assessment

This spec is well-structured and demonstrates clear thinking about the GPUI Element integration. The goal is appropriately scoped for a v2 card. The three MEDIUM findings are all addressable with small spec edits — none require architectural rethink. The most important change is clarifying the ChartState-vs-ChartElement ownership boundary (finding 1), because an implementer reading the spec cold would not know how to split state between the two structs. The GPU test environment concern (finding 3) and error handling for device creation (finding 6) are practical issues that will surface during implementation if not addressed now. The biggest risk is the synchronous-render-in-paint assumption — it is likely fine for v2 charts, but the spec should make the assumption explicit and auditable rather than implicit.
