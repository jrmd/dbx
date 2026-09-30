"use client";
import * as React from "react";
import { cn } from "./utils";
import { motion, useReducedMotion, type HTMLMotionProps } from "motion/react";
export function ScrollReveal({
  className,
  children,
  ...props
}: HTMLMotionProps<"div">) {
  const reduce = useReducedMotion();
  return (
    <motion.div
      initial={reduce ? false : { y: 28, opacity: 0.3 }}
      whileInView={{ y: 0, opacity: 1 }}
      viewport={{ once: true, amount: 0.2 }}
      transition={{ duration: 0.6, ease: [0.16, 1, 0.3, 1] }}
      className={cn("", className)}
      {...props}
    >
      {children}
    </motion.div>
  );
}
