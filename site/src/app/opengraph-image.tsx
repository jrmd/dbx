import { ImageResponse } from "next/og";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

export const alt = "DBX — A native database client";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default async function Image() {
  const [logo, screenshot] = await Promise.all([
    readFile(join(process.cwd(), "public/logo.png"), "base64"),
    readFile(join(process.cwd(), "public/workbench.png"), "base64"),
  ]);

  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          background:
            "radial-gradient(ellipse 70% 60% at 50% 0%, #1e3a8a 0%, #07090c 70%)",
          color: "#f1f5f9",
          padding: "64px 72px 0",
          fontFamily: "sans-serif",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
          <img src={`data:image/png;base64,${logo}`} width={52} height={52} alt="" />
          <span style={{ fontSize: 34, fontWeight: 600 }}>DBX</span>
        </div>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            marginTop: 36,
            fontSize: 68,
            lineHeight: 1,
            letterSpacing: "-0.04em",
          }}
        >
          <span>A native database client</span>
          <span style={{ color: "#94a3b8", fontSize: 40, marginTop: 18 }}>
            SQL · Documents · Search · Streams
          </span>
        </div>
        <div
          style={{
            display: "flex",
            marginTop: 44,
            borderRadius: 20,
            border: "1px solid #ffffff22",
            padding: 10,
            background: "#ffffff08",
          }}
        >
          <img
            src={`data:image/png;base64,${screenshot}`}
            width={1036}
            height={662}
            style={{ borderRadius: 12 }}
            alt=""
          />
        </div>
      </div>
    ),
    size,
  );
}
