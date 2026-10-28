# Domain docs

Ripscope uses a single context: the root `GLOSSARY.md` is the domain glossary, and
`docs/adr/` holds architecture decisions.

## Before exploring or changing domain behaviour

1. Read `GLOSSARY.md` and use its defined terms in code, tests, specs and tickets.
2. Read `docs/adr/README.md` and the decisions relevant to the work. Follow each
   superseded decision to its replacement; retained decisions are not all current.

## Keeping the model current

When work exposes an incorrect or missing domain term or decision, use
`domain-modeling` to update the glossary or relevant decision in the same change.
Check an apparent vocabulary gap against the existing terms before adding one.

Surface a conflict with an accepted ADR by number and title, and follow the
decision and review rules in `AGENTS.md` before changing the decision. Keep the
existing layout; add documentation when the work resolves a real term or decision.
