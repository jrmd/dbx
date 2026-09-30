# dbx.jrmd.dev

The DBX marketing site. Next.js 16, React 19, Tailwind 4, with motion and WebGL pieces from [Jez UI](https://ui.jrmd.dev).

```bash
pnpm install
pnpm dev
```

## Downloads

`src/lib/release.ts` reads the latest **published** GitHub release of `jrmd/dbx` and revalidates hourly. It links to the asset matching `*macos-arm64.zip` (plus its `.sha256`), and to a `*linux*.tar.gz` if one is ever attached. Drafts and prereleases are ignored by GitHub's `latest` endpoint, so until a release is published the download buttons point at the releases page.

Set `GITHUB_TOKEN` in the Vercel project to avoid the unauthenticated API rate limit (a fine-grained token with no extra permissions is enough).

## Deploying to Vercel

1. Import `jrmd/dbx` in Vercel and set **Root Directory** to `site`. The framework preset is detected as Next.js.
2. Add the domain `dbx.jrmd.dev`, then create the `CNAME` record Vercel shows for it (`dbx` → `cname.vercel-dns.com`).
3. Optional: under **Git → Ignored Build Step**, use `git diff --quiet HEAD^ HEAD -- .` so Rust-only commits don't redeploy the site.

## Jez UI components

Components live in `src/components/jez-ui/ui` and were added from the registry:

```bash
pnpm dlx shadcn@4.0.8 add https://ui.jrmd.dev/r/spotlight-card.json
```

Screenshots in `public/` are copies of `docs/screenshots/`; refresh them when the README screenshots change.
