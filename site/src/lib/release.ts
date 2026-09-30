export const REPO = "jrmd/dbx";
export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASES_URL = `${REPO_URL}/releases`;

export type Download = {
  name: string;
  url: string;
  size: number;
  checksumUrl?: string;
};

export type Release = {
  version: string;
  url: string;
  publishedAt: string;
  macos?: Download;
  linux?: Download;
};

type GitHubAsset = {
  name: string;
  browser_download_url: string;
  size: number;
};

type GitHubRelease = {
  tag_name: string;
  html_url: string;
  published_at: string;
  assets: GitHubAsset[];
};

function pick(assets: GitHubAsset[], pattern: RegExp): Download | undefined {
  const asset = assets.find((a) => pattern.test(a.name));
  if (!asset) return undefined;
  const checksum = assets.find((a) => a.name === `${asset.name}.sha256`);
  return {
    name: asset.name,
    url: asset.browser_download_url,
    size: asset.size,
    checksumUrl: checksum?.browser_download_url,
  };
}

/**
 * The newest published release and its platform downloads. Drafts and
 * prereleases are not returned by GitHub's `latest` endpoint, so this is
 * `null` until a release is made public; callers fall back to the releases
 * page. Revalidated hourly so a new release shows up without a redeploy.
 */
export async function getLatestRelease(): Promise<Release | null> {
  try {
    const headers: HeadersInit = { Accept: "application/vnd.github+json" };
    if (process.env.GITHUB_TOKEN) {
      headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
    }
    const response = await fetch(
      `https://api.github.com/repos/${REPO}/releases/latest`,
      { headers, next: { revalidate: 3600 } },
    );
    if (!response.ok) return null;
    const release = (await response.json()) as GitHubRelease;
    return {
      version: release.tag_name.replace(/^v/, ""),
      url: release.html_url,
      publishedAt: release.published_at,
      macos: pick(release.assets, /macos-arm64\.zip$/),
      linux:
        pick(release.assets, /linux-x86_64\.AppImage$/) ??
        pick(release.assets, /linux.*\.tar\.gz$/),
    };
  } catch {
    return null;
  }
}

export function formatSize(bytes: number) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
