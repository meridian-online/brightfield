## Workflow (orbit)

This project uses the orbit workflow: Card → Design → Spec → Implement → Review → Ship.

- `/orb:card` — capture a feature need with expected behaviours
- `/orb:distill` — extract capability cards from source material
- `/orb:discovery` — explore a vague idea through Socratic Q&A
- `/orb:design` — refine a feature card into technical decisions
- `/orb:spec` — crystallise interview into a structured specification
- `/orb:review-spec` — stress-test the spec before implementation
- `/orb:review-pr` — verify the PR against the spec's acceptance criteria

Artefacts live in `orbit/cards/`, `orbit/specs/`, and `orbit/decisions/`.

## Current Sprint

goal: "<sprint objective>"

cards:
  - NNNN: "<card goal>"
