"use client";
import * as React from "react";
import { cn } from "./utils";
export function SpotlightCard({
  children,
  className,
  ...rootProps
}: Omit<
  React.ComponentProps<"div">,
  keyof {
    children: React.ReactNode;
    className?: string;
  }
> & {
  children: React.ReactNode;
  className?: string;
}) {
  const [point, setPoint] = React.useState({ x: 50, y: 50 });
  return (
    <div
      {...rootProps}
      onPointerMove={(e) => {
        rootProps.onPointerMove?.(e);
        if (e.defaultPrevented) return;

        const r = e.currentTarget.getBoundingClientRect();
        setPoint({
          x: ((e.clientX - r.left) / r.width) * 100,
          y: ((e.clientY - r.top) / r.height) * 100,
        });
      }}
      className={cn(
        "relative overflow-hidden rounded-xl border border-border p-8",
        className,
      )}
    >
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 opacity-20"
        style={{
          background: `radial-gradient(circle at ${point.x}% ${point.y}%,var(--primary),transparent 65%)`,
        }}
      />
      <div className="relative">{children}</div>
    </div>
  );
}
