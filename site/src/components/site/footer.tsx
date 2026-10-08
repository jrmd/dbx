import Image from "next/image";
import { REPO_URL, RELEASES_URL } from "@/lib/release";

const links = [
  { href: REPO_URL, label: "GitHub" },
  { href: RELEASES_URL, label: "Releases" },
  { href: `${REPO_URL}/issues`, label: "Issues" },
  { href: `${REPO_URL}/blob/main/docs/user-guide.md`, label: "User guide" },
  { href: `${REPO_URL}/blob/main/docs/capability-matrix.md`, label: "Capabilities" },
];

export function Footer() {
  return (
    <footer className="border-t border-white/[.06]">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <Image src="/logo.png" alt="" width={24} height={24} />
          <span className="text-sm text-muted-foreground">
            DBX · MIT licensed · Made by{" "}
            <a href="https://jrmd.dev" className="text-foreground hover:underline">
              jrmd
            </a>
          </span>
        </div>
        <nav className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
          {links.map((link) => (
            <a
              key={link.label}
              href={link.href}
              className="transition-colors hover:text-foreground"
            >
              {link.label}
            </a>
          ))}
        </nav>
      </div>
    </footer>
  );
}
