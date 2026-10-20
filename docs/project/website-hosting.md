# Website hosting and approval

The website is a static-assets-only Cloudflare Worker. There is no Worker script,
analytics or runtime dependency on another origin. The landing page has a strict
same-origin Content Security Policy. Documentation also permits mdBook's inline
bootstrap and same-origin contents frame, including its navigation without
JavaScript. Diagrams are static SVGs. Security headers come from `site/_headers`.

Pushes, repository visibility changes, tags and releases never trigger deployment.
Cloudflare Workers Builds must stay disconnected. `workers_dev` and `preview_urls`
are disabled. Running a local build or dry run grants no publication authority.

## Local verification

Build with `scripts/build-site.sh --dev`, then run:

```sh
npm ci --ignore-scripts --no-audit --no-fund --prefix tools/site
env WRANGLER_SEND_METRICS=false node tools/site/node_modules/wrangler/bin/wrangler.js deploy --dry-run --config wrangler.jsonc
```

The manual Website verification workflow runs tooling checks, the combined build,
internal link checks and a hosting dry run. Its optional external-link check can
fail because of remote availability. It uploads a short-lived verification artifact
and never publishes a website.

## Future deployment setup

Complete setup only after the maintainer explicitly authorizes deployment:

1. Select the reviewed commit on `main`. Website changes deploy without a new
   release, as long as the root package version is already tagged. Building a
   commit does not deploy it.
2. Create the GitHub environment named `site-production`. Require a reviewer,
   including the maintainer for this solo-maintained project. Allow self-review
   (`prevent_self_review=false`) so the maintainer can approve a manually dispatched
   deployment. Disable administrator bypass in the environment settings.
3. Configure the environment's deployment branch policy to permit the intended
   dispatch branch, `main`. The selected commit is checked out separately.
4. Store a scoped `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` in that environment.
   The token needs only the permissions necessary to deploy the approved Worker and
   its custom domain. Keep credentials out of repository files and build artifacts.
5. Only after the explicit go, set the repository variable `SITE_DEPLOY_ENABLED`
   to `true`. It is intentionally unset by default. The deployment also requires a
   public repository, the reviewed commit's full SHA, the typed phrase
   `Deploy ripscope.com`, and approval in `site-production`.

The independent preflight job has no environment or provider credentials. It uses
the read-only workflow token to fetch the existing environment and verify a valid,
nonempty required-reviewer rule. A denied, missing or malformed response fails
closed. It does not create or repair protection. Administrator bypass needs a manual
settings check because this API response does not reliably expose that setting.

Before the first public deployment, run `scripts/build-site.sh --dev --check-external`
and inspect its diagnostics. External sites can reject automated requests or be
temporarily unavailable, so this audit is separate from the deployment job's
mandatory internal-link checks. Verify the owned repository, release downloads
and served installation commands after their access changes; checking HTML links
does not execute or validate download URLs printed inside code examples. Resolve
outstanding destinations before presenting them as available.

After preflight and approval, the deploy job checks out the selected commit,
verifies that it is on `main`, clean and at a tagged version, builds with
`--deploy`, checks internal links, and validates the Wrangler configuration before
deploying. Disable the repository
opt-in again when the approved deployment window has ended. Custom-domain routing
can affect DNS; the future authorization must include that change.

Setup is a prerequisite, not a completed deployment. This repository preparation
does not configure environments, secrets, provider resources, routes or DNS.

Provider reference: [Cloudflare static asset headers](https://developers.cloudflare.com/workers/static-assets/headers/).
Approval reference: [GitHub environment REST response](https://docs.github.com/en/rest/deployments/environments#get-an-environment).
