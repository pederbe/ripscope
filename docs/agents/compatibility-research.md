# Cscope compatibility research

## Target

Use a source-guided, output-verified method to define ripscope's compatibility requirements. The target is cscope's observable external contract for equivalent inputs and workflows, plus explicitly documented and approved differences.

Cscope's manual, tests, and source explain intended behavior, mechanisms, and useful test boundaries. They are evidence, not an implementation blueprint. Design ripscope's modules, data model, algorithms, and function boundaries around its own requirements using idiomatic Rust and independently justified correctness, maintainability, and performance choices. Mirror a cscope internal mechanism only when it is independently the best design for ripscope.

## Procedure

1. Reuse retained evidence when the relevant cscope source, reference profile, configuration, and claim remain unchanged. Confirm that the cited evidence covers the present claim; do not repeat source inspection or executable captures merely to recreate an unchanged result.
2. For new, uncertain, or disputed behavior, inspect cscope 15.9's manual (`doc/cscope.1` in its source release), relevant tests and prior evidence under `tests/fixtures` and `evidence/`, then the relevant mechanism in the official cscope 15.9 source. Record the source commit, paths, control flow, and build-dependent branches that bear on the claim.
3. Derive the smallest discriminating tests from that evidence. Include controls and boundaries that could disprove the source-derived interpretation instead of testing only the expected path.
4. Verify observable claims against the appropriate self-checked cscope reference executable. For the Ubuntu baseline, use the package and executable hash that `evidence/README.md` names. A capture must check rather than assume the executable identity. Record the reference profile, including package or version, executable hash, platform, configuration, inputs, command arguments, environment, and relevant locale. A different profile is separate evidence.
5. Record manual and source evidence, executable evidence, the reference profile, and remaining uncertainty separately. Source inspection does not prove how a differently patched or configured binary behaves, and one executable observation does not establish every cscope build.
6. Express the ripscope requirement as observable input, output, error, ordering, lifecycle, or workflow behavior, or as an explicitly approved difference. Do not express compatibility requirements as cscope function boundaries, data structures, or algorithms.

## Completion gate

A new compatibility conclusion is complete only when its retained notes:

- identify the claim and the existing evidence reused or the reason new evidence was required;
- cite the relevant manual, tests, source commit and mechanism where they apply;
- preserve the discriminating executable evidence and self-checked reference profile where the claim is observable;
- include a control or boundary capable of disproving the interpretation;
- separate source-derived interpretation, observed output, and unresolved uncertainty; and
- state the ripscope requirement as observable behavior or an explicitly approved difference.

When relevant source or executable verification is unavailable, record the attempts and limitation and keep the conclusion provisional. Compatibility corrections and limitations require explicit evidence and approval.
