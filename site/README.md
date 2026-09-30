# dbx.jrmd.dev

The DBX marketing site. Next.js 16, React 19, Tailwind 4, with motion and WebGL pieces from [Jez UI](https://ui.jrmd.dev).

```bash
pnpm install
pnpm dev
```

## Downloads

`src/lib/release.ts` reads the latest **published** GitHub release of `jrmd/dbx` and revalidates hourly. It links to the asset matching `*macos-arm64.zip` (plus its `.sha256`), and to a `*linux*.tar.gz` if one is ever attached. Drafts and prereleases are ignored by GitHub's `latest` endpoint, so until a release is published the download buttons point at the releases page.

Set `GITHUB_TOKEN` in the Vercel project to avoid the unauthenticated API rate limit (a fine-grained token with no extra permissions is enough).

## Deployment

The Vercel project `dbx` is connected to `jrmd/dbx` with **Root Directory** `site`, so every push to `main` that touches `site/` deploys to production. The ignored build step `git diff --quiet HEAD^ HEAD -- .` skips commits that only change the app. `dbx.jrmd.dev` is a Cloudflare `CNAME` to `cname.vercel-dns.com`.

## Jez UI components

Components live in `src/components/jez-ui/ui` and were added from the registry:

```bash
pnpm dlx shadcn@4.0.8 add https://ui.jrmd.dev/r/spotlight-card.json
```

Screenshots in `public/` are copies of `docs/screenshots/`; refresh them when the README screenshots change.
