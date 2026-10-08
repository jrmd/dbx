"use client";

import * as React from "react";
import Image from "next/image";
import {
  motion,
  useReducedMotion,
  useScroll,
  useTransform,
} from "motion/react";
import { WebGLRibbonField } from "@/components/jez-ui/ui/webgl-ribbon-field";
import { SplitText } from "@/components/jez-ui/ui/split-text";
import { REPO_URL, RELEASES_URL, type Release } from "@/lib/release";
import { AppleIcon, ArrowUpRight, GitHubIcon, LinuxIcon } from "./icons";

type Platform = "mac" | "linux" | "other";

function usePlatform(): Platform {
  return React.useSyncExternalStore(
    () => () => {},
    () => {
      const agent = navigator.userAgent;
      if (/Mac/i.test(agent)) return "mac";
      if (/Linux|X11/i.test(agent) && !/Android/i.test(agent)) return "linux";
      return "other";
    },
    () => "mac",
  );
}

export function Hero({ release }: { release: Release | null }) {
  const platform = usePlatform();
  const reduce = useReducedMotion();
  const frame = React.useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({
    target: frame,
    offset: ["start end", "start 0.25"],
  });
  const rotateX = useTransform(scrollYProgress, [0, 1], [18, 0]);
  const scale = useTransform(scrollYProgress, [0, 1], [0.94, 1]);

  const version = release?.version ?? "0.1.0";
  const macUrl = release?.macos?.url ?? RELEASES_URL;

  return (
    <section className="relative overflow-hidden">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 h-[760px] opacity-45 [mask-image:linear-gradient(to_bottom,black_25%,transparent_90%)]"
      >
        <WebGLRibbonField
          color="#3b82f6"
          speed={0.6}
          label="Flowing blue ribbons"
          className="h-full rounded-none"
          style={{ background: "transparent" }}
        />
      </div>
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_60%_40%_at_50%_0%,#2563eb33,transparent_70%)]"
      />

      <div className="relative mx-auto max-w-6xl px-6 pt-20 pb-10 text-center sm:pt-28">
        <a
          href={release?.url ?? RELEASES_URL}
          className="inline-flex h-8 items-center gap-2 rounded-full border border-white/10 bg-white/[.04] px-3.5 text-[13px] text-muted-foreground backdrop-blur transition-colors hover:border-white/20 hover:text-foreground"
        >
          <span className="text-foreground">v{version} Preview</span>
          <span className="h-3.5 w-px bg-white/15" />
          Free and open source
          <ArrowUpRight className="size-3.5" />
        </a>

        <h1 className="mx-auto mt-8 max-w-4xl text-[clamp(2.75rem,7.5vw,5.75rem)] leading-[.95] font-medium tracking-[-0.045em] text-balance">
          <SplitText>A native database client</SplitText>
        </h1>

        <p className="mx-auto mt-7 max-w-xl text-lg leading-relaxed text-pretty text-muted-foreground">
          Browse, query, and edit your databases in one app. Written in Rust and drawn on the GPU. Free and open source.
        </p>

        <div className="mt-10 flex flex-col items-center justify-center gap-3 sm:flex-row">
          {platform === "linux" ? (
            <a
              href={release?.linux?.url ?? "#download"}
              className="inline-flex h-12 items-center gap-2.5 rounded-full bg-primary px-6 font-medium text-white shadow-[inset_0_1px_0_#ffffff33,0_8px_32px_-8px_#2563eb] transition-colors hover:bg-[#1d55d4]"
            >
              <LinuxIcon className="size-5" />
              {release?.linux ? "Download for Linux" : "Build for Linux"}
            </a>
          ) : (
            <a
              href={platform === "mac" ? macUrl : "#download"}
              className="inline-flex h-12 items-center gap-2.5 rounded-full bg-primary px-6 font-medium text-white shadow-[inset_0_1px_0_#ffffff33,0_8px_32px_-8px_#2563eb] transition-colors hover:bg-[#1d55d4]"
            >
              <AppleIcon className="size-5" />
              Download for macOS
            </a>
          )}
          <a
            href={REPO_URL}
            className="inline-flex h-12 items-center gap-2.5 rounded-full border border-white/10 bg-white/[.04] px-6 font-medium backdrop-blur transition-colors hover:border-white/20 hover:bg-white/[.07]"
          >
            <GitHubIcon className="size-[18px]" />
            Star on GitHub
          </a>
        </div>
        <p className="mt-5 text-[13px] text-muted-foreground/80">
          {platform === "linux"
            ? "Wayland or X11 · Vulkan · MIT licensed"
            : "Apple Silicon · Notarized by Apple · MIT licensed"}
        </p>
      </div>

      <div
        ref={frame}
        className="relative mx-auto max-w-6xl px-4 pb-24 sm:px-6"
        style={{ perspective: 1400 }}
      >
        <motion.div
          style={reduce ? undefined : { rotateX, scale, transformOrigin: "top" }}
          className="relative rounded-[20px] border border-white/10 bg-white/[.03] p-2 shadow-[0_40px_120px_-30px_#2563eb66,0_0_0_1px_#00000080] backdrop-blur-sm sm:p-3"
        >
          <Image
            src="/workbench.png"
            alt="DBX showing tabbed tables, a schema explorer, foreign-key links, and a row inspector"
            width={2304}
            height={1472}
            priority
            sizes="(min-width: 1152px) 1128px, 100vw"
            className="rounded-[12px]"
          />
        </motion.div>
      </div>
    </section>
  );
}
