# Website hosting and deployment

Ripscope's website is a static-assets-only Cloudflare Worker. There is no Worker
script, analytics or runtime dependency on another origin. Security headers come
from `site/_headers`, and diagrams are committed SVGs.

Follow the [site-tool guide](../../tools/site/README.md) to build the landing page
and book, check links and assets, or validate hosting without publishing. It also
explains how production builds select documentation and release download links,
and how to retry website delivery independently of a release.

[wrangler.jsonc](../../wrangler.jsonc) owns static asset routing and hosting
options. Cloudflare Workers Builds handles production deployment. Local builds
and the [verification workflow](../../.github/workflows/docs.yml) do not deploy.
