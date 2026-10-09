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
      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10 lg:flex-row lg:items-center lg:justify-between">
        <a
          href="https://bonusobjective.com"
          className="flex w-fit items-center gap-3 rounded-md text-sm text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-foreground"
        >
          <Image src="/bonus-objective.svg" alt="" width={32} height={32} />
          <span>A bonus objective project</span>
        </a>
        <nav aria-label="Footer" className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
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
