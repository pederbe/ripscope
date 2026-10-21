# Website hosting and deployment

The website is a static-assets-only Cloudflare Worker at ripscope.com. There is
no Worker script, analytics or runtime dependency on another origin. Security
headers come from `site/_headers`. Diagrams are committed static SVGs.

## Automatic deployment

Cloudflare Workers Builds is connected to `pederbe/ripscope`, production branch
`main`. Website-input changes build and deploy automatically. No manual workflow,
typed confirmation, GitHub environment approval or release is needed for a website
update. Unrelated application and CI changes do not trigger website deployment.

The production build settings are:

- Build command: `bash scripts/build-site.sh --cloudflare`.
- Deploy command: `node tools/site/node_modules/wrangler/bin/wrangler.js deploy`.
- Root directory: `/`.
- Build variable: `SKIP_DEPENDENCY_INSTALL=1`.
- Preview builds: disabled.

Build watch paths cover `docs/*`, `site/*`, `tools/site/*`,
`scripts/build-site.sh`, `book.toml`, `wrangler.jsonc`, `Cargo.toml`, both root
installers, `CHANGELOG.md`, `CONTRIBUTING.md`, `CONTEXT.md`, `evidence/*`, and the
canonical fixture directories referenced by `evidence/sources.json`. Those fixtures
are published website evidence. Keep watch paths in sync when adding a source
outside these locations.

The build installs Rust when absent, downloads the checked mdBook binary, installs
locked Node tooling and runs the existing deployment build with internal link
checks. The package version must already have its release tag because download
links use that version. This does not require a new release for website changes.
`workers_dev` and `preview_urls` remain disabled.

## Local verification

Build locally with `scripts/build-site.sh --dev`. To check hosting without
publishing:

```sh
npm ci --ignore-scripts --no-audit --no-fund --prefix tools/site
env WRANGLER_SEND_METRICS=false node tools/site/node_modules/wrangler/bin/wrangler.js deploy --dry-run --config wrangler.jsonc
```

The optional Website verification workflow checks tooling, builds the site and
validates hosting without publishing. Local builds do not deploy. For an explicit
rebuild of production without a source change, retry the build in Cloudflare.

[Cloudflare build watch paths](https://developers.cloudflare.com/workers/ci-cd/builds/build-watch-paths/)
control which changes start builds.
