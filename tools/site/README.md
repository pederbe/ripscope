# Static website build

## Landing page styles

The landing page uses Tailwind CSS 4 and daisyUI 5 with the existing Ripscope
palette. CSS is compiled at edit time and committed, so normal site builds need
neither Node.js nor a browser-side CSS compiler. After changing landing markup,
utilities or the theme in `tools/site/landing.css`, run:

```sh
npm ci --ignore-scripts --no-audit --no-fund --prefix tools/site
npm run build:landing --prefix tools/site
```

Commit the generated `site/landing.css` with its inputs. Components and utilities
are served locally under the existing Content Security Policy. The landing page
uses a golden-yellow highlight against navy, with cyan as its second accent.
The scope scan, ambient glow and update diagram animate while visible. Scroll
entrances are prepared before the first paint and run once as content enters
the viewport, with 1.2-second movement and a short stagger. The page-wide motion
control pauses the decorative effects and editor illustration. The illustration also pauses on
hover and focus. All loops pause outside the viewport and in hidden tabs.
Reduced-motion preferences retain a static page and the complete final result.

## Combined build

Run `scripts/build-site.sh --dev` from a development checkout. The combined output
is `target/site`, with the landing page at `/` and documentation at `/docs/`.
The build requires Rust and mdBook 0.5.4 (`MDBOOK` can select an installed binary).
It never deploys. `--docs-only` checks documentation during page development;
it cannot be combined with a deployment build.

`scripts/build-site.sh --deploy` requires a clean checkout whose root package version
already has its `vVERSION` tag, so the page's download links exist. Cloudflare
Workers Builds runs this build automatically for website-input changes on `main`,
so website changes ship without a new release or manual deployment.
`scripts/build-site.sh --cloudflare` installs the build tools and runs `--deploy`
in Cloudflare's build image. Local `--dev` and `--deploy` builds do not publish.

Only chapters selected by `docs/SUMMARY.md`, their local assets, resolved Markdown
includes and the theme enter the documentation build. `--list-inputs` lists the
documentation source closure. It does not
include general repository files, landing assets or build tools. Internal link and
anchor checks are mandatory. Add `--check-external` to check remote responses with
an installed `lychee`; temporary network failures are reported as failures.

The required `evidence/sources.json` version 1 contains only `version`, `metadata`
and `mappings`. Its metadata array names individual files under `evidence/`, and
each mapping names one canonical fixture file and its path under
`evidence/captures/`. Directory sources are refused. The build copies only those
declared files and the map itself to
`/evidence/` before checking links, including during `--docs-only` builds.
`--list-inputs` includes the map, metadata and original sources. Capture copies
are generated from the canonical fixtures; their aliases are never maintained
separately. Unexpected files are omitted even when tracked; physical capture
aliases are ignored.
The paths `site/docs` and `site/evidence` are reserved for generated output.

Normal builds use the committed SVG diagrams without a browser or Mermaid script
in the delivered pages. When diagram source or themes change, install the optional
web tooling with `npm ci --ignore-scripts --prefix tools/site`, make a Puppeteer
Chrome available, and run `scripts/build-site.sh --dev --render-diagrams`.
`RIPSCOPE_MMDC` can select an existing Mermaid CLI. Inspect the generated dark and
light SVGs, remove obsolete renders identified by the build, and commit them.
Each Mermaid fence needs a following `<!-- description: ... -->` caption that
explains the relationships in text. JavaScript here is the Mermaid web renderer's
build-time ecosystem; the site builder is Rust.

Run the tooling checks with:

```sh
cargo fmt --manifest-path tools/site/Cargo.toml --check
cargo clippy --locked --all-targets --manifest-path tools/site/Cargo.toml -- -D warnings
cargo test --locked --manifest-path tools/site/Cargo.toml
```

Combined development and deployment builds also copy the canonical root `install.sh`
and `install.ps1` to the site root. Documentation-only builds omit those scripts.
`--list-inputs` includes both canonical installers for the public source closure.
They must be regular files; symlinks and `site/install.sh` / `site/install.ps1`
shadow copies are refused. Keep only the root sources when changing installation.

Static hosting configuration and automatic deployment are described in
[Website hosting and deployment](../../docs/project/website-hosting.md).

The optional tooling lock retains patched KaTeX and scopes Sharp 0.35.5 and
Undici 7.29.1 overrides to Miniflare. These patch versions correct advisories in
its pinned dependencies while retaining the selected Wrangler version. They affect
local hosting and deployment tooling, rather than files served to visitors.
