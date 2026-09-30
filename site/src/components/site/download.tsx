import { ScrollReveal } from "@/components/jez-ui/ui/scroll-reveal";
import {
  REPO_URL,
  RELEASES_URL,
  formatSize,
  type Release,
} from "@/lib/release";
import { AppleIcon, ArrowUpRight, LinuxIcon } from "./icons";

function Code({ lines }: { lines: string[] }) {
  return (
    <pre className="overflow-x-auto rounded-xl border border-white/[.07] bg-[#0a0c10] p-4 font-mono text-[13px] leading-6">
      {lines.map((line) => (
        <div key={line}>
          <span className="text-muted-foreground select-none">$ </span>
          {line}
        </div>
      ))}
    </pre>
  );
}

export function Download({ release }: { release: Release | null }) {
  const mac = release?.macos;
  const macIntel = release?.macosIntel;
  const linux = release?.linux;
  const version = release?.version ?? "0.1.0";

  return (
    <section
      id="download"
      className="border-t border-white/[.06] bg-[#0a0c10] py-28 sm:py-36"
    >
      <div className="mx-auto max-w-6xl px-6">
        <div className="max-w-2xl">
          <h2 className="text-[clamp(2.25rem,5vw,3.75rem)] leading-[1] font-medium tracking-[-0.04em]">
            Download DBX
          </h2>
          <p className="mt-6 text-lg leading-relaxed text-muted-foreground">
            MIT licensed. No account and no telemetry. Download a build or
            compile it yourself.
          </p>
        </div>

        <ScrollReveal className="mt-14 grid gap-4 lg:grid-cols-2">
          <div className="flex min-w-0 flex-col rounded-[20px] border border-white/[.07] bg-card p-6 sm:p-8">
            <AppleIcon className="size-8" />
            <h3 className="mt-6 text-2xl font-medium tracking-tight">macOS</h3>
            <p className="mt-2 text-muted-foreground">
              {macIntel ? "Apple Silicon and Intel." : "Apple Silicon."} Signed
              with Developer ID and notarized by Apple.
            </p>
            <div className="mt-auto pt-10">
              <a
                href={mac?.url ?? RELEASES_URL}
                className="inline-flex h-12 items-center gap-2.5 rounded-full bg-primary px-6 font-medium text-white shadow-[inset_0_1px_0_#ffffff33,0_8px_32px_-8px_#2563eb] transition-colors hover:bg-[#1d55d4]"
              >
                <AppleIcon className="size-5" />
                Download DBX {version}
              </a>
              <p className="mt-4 text-sm text-muted-foreground">
                {mac ? (
                  <>
                    {mac.name} · {formatSize(mac.size)}
                    {mac.checksumUrl && (
                      <>
                        {" · "}
                        <a
                          href={mac.checksumUrl}
                          className="underline decoration-white/20 underline-offset-4 hover:text-foreground"
                        >
                          SHA-256
                        </a>
                      </>
                    )}
                  </>
                ) : (
                  "Unzip and move DBX.app into Applications."
                )}
              </p>
              {macIntel && (
                <a
                  href={macIntel.url}
                  className="mt-2 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
                >
                  Download for Intel Macs · {formatSize(macIntel.size)}
                  <ArrowUpRight className="size-3.5" />
                </a>
              )}
            </div>
          </div>

          <div className="flex min-w-0 flex-col rounded-[20px] border border-white/[.07] bg-card p-6 sm:p-8">
            <LinuxIcon className="size-8" />
            <h3 className="mt-6 text-2xl font-medium tracking-tight">Linux</h3>
            <p className="mt-2 text-muted-foreground">
              {linux
                ? "An x86_64 AppImage for Wayland or X11 with a Vulkan driver. Or build it yourself with Rust 1.97 or newer."
                : "Wayland or X11 with a Vulkan driver. Build it with Rust 1.97 or newer."}
            </p>
            <div className="mt-auto space-y-4 pt-10">
              {linux && (
                <div className="pb-2">
                  <a
                    href={linux.url}
                    className="inline-flex h-12 items-center gap-2.5 rounded-full border border-white/10 bg-white/[.06] px-6 font-medium transition-colors hover:border-white/20 hover:bg-white/[.1]"
                  >
                    <LinuxIcon className="size-5" />
                    Download DBX {version}
                  </a>
                  <p className="mt-4 text-sm text-muted-foreground">
                    {linux.name} · {formatSize(linux.size)}
                    {linux.name.endsWith(".AppImage") &&
                      " · Mark it executable, then run it."}
                  </p>
                </div>
              )}
              <Code
                lines={[
                  "git clone https://github.com/jrmd/dbx.git && cd dbx",
                  "make linux-run",
                ]}
              />
              <a
                href={`${REPO_URL}#build-from-source`}
                className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
              >
                Build instructions and dependencies
                <ArrowUpRight className="size-3.5" />
              </a>
            </div>
          </div>
        </ScrollReveal>

        <p className="mt-8 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          DBX is a preview and in active development. Point it at a disposable
          database and a least-privilege account while you try it out.
        </p>
      </div>
    </section>
  );
}
