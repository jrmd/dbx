"use client";
import * as React from "react";
import { cn } from "./utils";
import { motion, useReducedMotion } from "motion/react";
export function SplitText({
  children,
  className,
  ...rootProps
}: Omit<
  React.ComponentProps<"span">,
  keyof {
    children: string;
    className?: string;
  }
> & {
  children: string;
  className?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <span
      {...rootProps}
      aria-label={children}
      className={cn("inline-block", className)}
    >
      {children.split(" ").map((word, i) => (
        <motion.span
          aria-hidden="true"
          key={i}
          className="mr-[.25em] inline-block"
          initial={reduce ? false : { y: 18, filter: "blur(5px)" }}
          whileInView={{ y: 0, filter: "blur(0px)" }}
          viewport={{ once: true }}
          transition={{ delay: i * 0.055, duration: 0.55 }}
        >
          {word}
        </motion.span>
      ))}
    </span>
  );
}
