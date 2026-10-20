# Ripscope media

The landing page contains an HTML/CSS illustration with real quick-start rows.
The standalone demo in the README is different. It shows four terminal screens
captured from an actual Vim session, with editorial holds for a 7.8-second loop.
The holds do not reflect the timing of typing or queries.

Both use the C sources and expected rows in `tools/media/fixture`. The workflow
starts with `greet.c` and `main.c` already selected and indexed, finds the caller
at `main.c:5`, adds and saves `helper` at `main.c:8`, then repeats
`:cs find c greet` straight away. Ripscope refreshes the index before that search,
so the saved caller is found with no index build in between. The capture
verifies both quickfix entries and byte-identical cscope connection records
before and after the saved edit. No reconnect is performed.

## Capture actual Vim

Native Linux capture requires Rust, Vim with cscope/quickfix support, tmux and an
existing Ripscope executable. It creates an isolated tmux server and exits it
on success or failure. It does not edit a user Vim configuration. Each output
directory must be new, with an existing parent; previous evidence is preserved.

```sh
cargo run --locked --manifest-path tools/media/Cargo.toml -- capture --binary /absolute/path/to/ripscope --output /tmp/ripscope-vim-capture
```

The output keeps exact screens and cursor locations, source bytes, raw quickfix
and connection records, backend result rows and `manifest.json`. The manifest
identifies the executable by version and SHA-256, the adapter, capture toolchain,
fixture and screen hashes. Checkout HEAD at capture is recorded separately and
is not a binary build receipt. A null backend build receipt remains explicit.
It establishes this small workflow, not broad navigation accuracy or performance.
The fixture's temporary SQLite files are removed after Vim exits.

The shipped demo's original screen and cursor bytes are retained in
`scripts/demo/capture`. Its public-safe navigation observation omits temporary
paths and process IDs, while recording exact connection equality and hashes of
the original private records. The capture manifest hashes that observation.

## Render the demo and link preview

Use Node.js 22 or later, the site's existing Puppeteer and cached Chrome, and
FFmpeg with libx264. `RIPSCOPE_PUPPETEER` selects an existing module entry point,
`BROWSER_PATH` an existing Chrome executable, and `FFMPEG_PATH` an existing FFmpeg.
Install no browser when an existing setup is available.

```sh
node scripts/demo/render.mjs scripts/demo/capture /tmp/ripscope-media
```

`--images-only` renders the four frame PNGs, poster, 1200x630 link preview and
manifest without encoding video. Rendering verifies screen hashes and shared
fixture rows first. It produces media in the new output directory, and never
replaces site assets automatically. Inspect all frames at original size before
copying the completed assets into `site/`.

Each 1200x675 frame uses the website's palette and shows a title, the Vim screen
and the list of steps, with the current step highlighted. To fit the frame, it shows the left 60
columns, which hold all of the screen text apart from the status lines' cursor
positions, and leaves out the same number of Vim's empty `~` rows from every
screen. The same `demo.gif` serves the README and posts.

| Asset | Format and purpose |
| --- | --- |
| `demo.mp4` | Silent H.264 video, 1200x676, 10 fps, 7.8 seconds |
| `demo.gif` | Looping GIF shown in the README, 1200x675, 10 fps |
| `demo-poster.png` | Final actual Vim screen, showing both callers |
| `demo-manifest.json` | Capture provenance, exact rows and editorial holds |
| `og.png` | 1200x630 link preview, showing the captured `main.c` and quickfix rows |
| `logo.svg`, `favicon.svg` | Standalone versions of the landing page's scope-reticle mark |

The SVG wordmark uses system fonts. The scope mark's circles, crosshair and
braces retain the landing page geometry. No external fonts or runtime media
requests are needed.

## Suggested media description

Ripscope in Vim. Find who calls greet, add and save helper in the already-selected
main.c, then search again and find callers at lines 5 and 8 through the same
connection, with no index build in between. This loop is rendered from captured
terminal screens with editorial holds, not a recording of elapsed query time.
