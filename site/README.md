# Site

The project's landing page: what the app is and a download for the
visitor's platform, in English and Spanish (`/es/`). Static Astro build
served by a Cloudflare Worker (`wrangler.jsonc`).

## Where the data comes from

- **Name, description, repository, address**: `site.config.json`. Set
  `site` to the real URL; `base` serves the page under a path of a shared
  domain (e.g. `"/myapp"`, plus the route in `wrangler.jsonc`).
- **What is offered**: `nix/targets.nix`, the matrix the release workflow
  builds. `scripts/data.mjs` evaluates it with `nix eval` into
  `src/data/targets.json`, which is committed: hosts without Nix (the
  Cloudflare builder) build from it, and CI fails when it drifts. Run
  `node scripts/data.mjs` after changing the targets.
- **Files, sizes, version and date**: the newest GitHub release with a
  `latest.json`, fetched at build time. In the browser the page asks
  GitHub again and switches to a newer release, so a page built before a
  release still offers it.

JavaScript only detects the platform, switches the download tabs and
runs the language menu; without it the page lists every file.

## Commands

```sh
cd site
pnpm install
pnpm dev        # http://localhost:4321/
pnpm build      # dist/
pnpm run deploy # build + wrangler deploy (needs Cloudflare credentials)
```

The release workflow's `site` job deploys after each published release
once the repository has a `CLOUDFLARE_API_TOKEN` secret (and
`CLOUDFLARE_ACCOUNT_ID` when the token covers more than one account).
Connecting the repository to Cloudflare's builder (root `site`, build
`pnpm run build`, deploy `npx wrangler deploy`) also works.

## Making it yours

The page is a hero and the download section. Add sections under
`src/components/` (screenshots with `astro:assets` `<Image>`), strings to
both languages in `src/i18n.ts`, and colours in `src/styles/site.css`
(`--accent` first).
