"use client";
import * as React from "react";
import { cn } from "./utils";
import { WebGLStage, type WebGLProps } from "./webgl-stage";
export function WebGLRibbonField({ className, ...props }: WebGLProps) {
  return <WebGLStage kind="ribbons" className={cn("", className)} {...props} />;
}
