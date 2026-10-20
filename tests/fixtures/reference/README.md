# Reference evidence

These files are the evidence that tests compare Ripscope with: observations of the
pinned cscope 15.9 reference executable. They were copied from the project's research
archive, which is not part of this repository. Archive paths in fixture provenance
fields name files in that archive.

| File | Archive origin | Original SHA-256 (LF) |
| --- | --- | --- |
| `cscope-baseline.json` | `experiments/cscope-baseline/results.json` | `c543a589ffe468a35043a329b5bc8b4c4c027da13e6a9237e0475d5fae9b903e` |
| `selection-oracle.json` | `workload-evidence/selection-oracle.json` | `ffc64d85a67b1dc833e2b8d4ff9d38f62242a9b971e1353a8ccfa0d632052a9a` |

The copies are unchanged except in one respect. The reference executable's absolute
path, which cscope also prints in its `--version` output, is replaced by `cscope` in
`cscope-baseline.json`. The same substitution applies to `../definition-eligibility.json`,
`../written-assignment-boundary.json` and `../written-message-pragma.json`. The
executable's identity stays recorded as `binary_sha256`.
