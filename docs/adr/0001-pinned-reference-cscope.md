# A pinned cscope executable defines expected results

Ripscope answers cscope's searches, so its expected results come from cscope
itself. cscope builds carry different patches and can answer the same search
differently, and its source does not always predict what a patched binary does.

## Decision

Expected cscope results come from one build: Ubuntu's `cscope 15.9-2build1` for
amd64, run with `LC_ALL=C`. The tools that run it check the executable's SHA-256
before every command. cscope's source is read to explain why it behaves as it does,
but only the executable's output counts as cscope's answer. Ripscope's own output is
never used as its expected result.

## Consequences

- Each fixture records where its expectation came from, so a reviewer can tell a
  captured cscope answer from an intended Correction.
- Output from another cscope build is separate evidence and does not replace a
  capture from the reference.
- Linux runs the reference directly and Windows runs it inside Ubuntu WSL. The
  [development tools](https://github.com/pederbe/ripscope/blob/main/scripts/README.md) describe the setup.
