import Image from "next/image";
import Link from "next/link";
import { REPO_URL } from "@/lib/release";
import { GitHubIcon } from "./icons";

const links = [
  { href: "#features", label: "Features" },
  { href: "#databases", label: "Databases" },
  { href: "#safety", label: "Safety" },
  { href: "#vault", label: "Vault" },
  { href: "#download", label: "Download" },
];

export function Nav() {
  return (
    <header className="sticky top-0 z-50 border-b border-white/[.06] bg-background/70 backdrop-blur-xl backdrop-saturate-150">
      <div className="mx-auto flex h-16 max-w-6xl items-center justify-between px-6">
        <Link href="/" className="flex items-center gap-2.5" aria-label="DBX home">
          <Image src="/logo.png" alt="" width={28} height={28} priority />
          <span className="text-[17px] font-semibold tracking-tight">DBX</span>
        </Link>
        <nav className="hidden items-center gap-8 text-sm text-muted-foreground md:flex">
          {links.map((link) => (
            <a
              key={link.href}
              href={link.href}
              className="transition-colors hover:text-foreground"
            >
              {link.label}
            </a>
          ))}
        </nav>
        <div className="flex items-center gap-2">
          <a
            href={REPO_URL}
            className="inline-flex h-9 items-center gap-2 rounded-full px-3 text-sm text-muted-foreground transition-colors hover:bg-white/[.06] hover:text-foreground"
          >
            <GitHubIcon className="size-4" />
            <span className="hidden sm:inline">GitHub</span>
          </a>
          <a
            href="#download"
            className="inline-flex h-9 items-center rounded-full bg-foreground px-4 text-sm font-medium text-background transition-colors hover:bg-white/85"
          >
            Download
          </a>
        </div>
      </div>
    </header>
  );
}
