import type { Metadata, Viewport } from "next";
import { Instrument_Sans, JetBrains_Mono } from "next/font/google";
import "./globals.css";

const instrument = Instrument_Sans({
  variable: "--font-instrument",
  subsets: ["latin"],
});

const jetbrains = JetBrains_Mono({
  variable: "--font-jetbrains",
  subsets: ["latin"],
});

const description =
  "A free, open-source database client for PostgreSQL, MySQL, SQLite, and Redis. Written in Rust and drawn on the GPU — no Electron, no webview.";

export const metadata: Metadata = {
  metadataBase: new URL("https://dbx.jrmd.dev"),
  title: "DBX — A native database client",
  description,
  applicationName: "DBX",
  keywords: [
    "database client",
    "PostgreSQL",
    "MySQL",
    "SQLite",
    "Redis",
    "Rust",
    "GPUI",
    "open source",
    "TablePlus alternative",
  ],
  openGraph: {
    title: "DBX — A native database client",
    description,
    url: "https://dbx.jrmd.dev",
    siteName: "DBX",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "DBX — A native database client",
    description,
  },
};

export const viewport: Viewport = {
  themeColor: "#07090c",
  colorScheme: "dark",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html
      lang="en"
      className={`${instrument.variable} ${jetbrains.variable} h-full antialiased`}
    >
      <body className="flex min-h-full flex-col">{children}</body>
    </html>
  );
}
