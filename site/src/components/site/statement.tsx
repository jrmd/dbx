"use client";

import * as React from "react";
import {
  motion,
  useReducedMotion,
  useScroll,
  useTransform,
  type MotionValue,
} from "motion/react";

const text =
  "DBX is built with GPUI, the Rust UI framework behind the Zed editor. There's no Electron, webview, or browser runtime underneath, and every frame is drawn on your GPU.";

function Word({
  children,
  progress,
  range,
}: {
  children: string;
  progress: MotionValue<number>;
  range: [number, number];
}) {
  const opacity = useTransform(progress, range, [0.18, 1]);
  return (
    <motion.span style={{ opacity }} className="mr-[.26em] inline-block">
      {children}
    </motion.span>
  );
}

export function Statement() {
  const ref = React.useRef<HTMLParagraphElement>(null);
  const reduce = useReducedMotion();
  const { scrollYProgress } = useScroll({
    target: ref,
    offset: ["start 0.85", "end 0.45"],
  });
  const words = text.split(" ");

  return (
    <section className="mx-auto max-w-5xl px-6 py-28 sm:py-40">
      <p
        ref={ref}
        aria-label={text}
        className="text-[clamp(1.75rem,4vw,3.25rem)] leading-[1.12] font-medium tracking-[-0.03em]"
      >
        {reduce
          ? text
          : words.map((word, i) => (
              <Word
                key={i}
                progress={scrollYProgress}
                range={[i / words.length, (i + 1) / words.length]}
              >
                {word}
              </Word>
            ))}
      </p>
    </section>
  );
}
