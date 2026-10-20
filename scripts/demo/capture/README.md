# Original replay inputs

These four screens and cursor positions are unchanged bytes captured from an
actual Vim session. `manifest.json` identifies their hashes, commands, source
bytes, exact backend rows and executable SHA-256. Its checkout revision is not
a build receipt for that executable.

`observation.json` retains the quickfix locations and text before and after
the saved edit. It records the observed byte equality and SHA-256 of the original
`cs show` connection records. Temporary absolute paths, process IDs and Vim's
internal buffer fields are omitted from this public projection. Hashes of the
original private records are retained for comparison, but their bytes are not
included here. The backend rows in `callers-before.txt` and `callers-after.txt`
are unmodified.

Render these retained inputs using `node scripts/demo/render.mjs scripts/demo/capture /tmp/ripscope-media`.
The demo holds frames for readability and makes no elapsed-time claim.
