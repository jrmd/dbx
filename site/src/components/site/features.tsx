import type { ReactNode } from "react";
import { SpotlightCard } from "@/components/jez-ui/ui/spotlight-card";
import { ScrollReveal } from "@/components/jez-ui/ui/scroll-reveal";
import { cn } from "@/lib/utils";

function Tile({
  title,
  body,
  children,
  className,
}: {
  title: string;
  body: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <SpotlightCard
      className={cn(
        "flex flex-col rounded-[20px] border-white/[.07] bg-card p-0 [&>div:last-child]:flex [&>div:last-child]:h-full [&>div:last-child]:flex-col",
        className,
      )}
    >
      <div className="p-7 pb-0 sm:p-8 sm:pb-0">
        <h3 className="text-xl font-medium tracking-tight">{title}</h3>
        <p className="mt-2 max-w-md leading-relaxed text-muted-foreground">
          {body}
        </p>
      </div>
      <div className="mt-auto pt-8">{children}</div>
    </SpotlightCard>
  );
}

const transports = [
  { name: "SSH tunnel", detail: "Jump hosts, agent or key, checked host keys" },
  { name: "Unix socket", detail: "Local or through the tunnel" },
  { name: "TLS", detail: "Certificates verified by default" },
  { name: "Cloud tokens", detail: "AWS RDS IAM and Azure Entra" },
];

function Transports() {
  return (
    <ul className="divide-y divide-white/[.06] border-t border-white/[.06]">
      {transports.map((transport) => (
        <li
          key={transport.name}
          className="flex items-baseline justify-between gap-4 px-7 py-4 sm:px-8"
        >
          <span className="text-lg font-medium tracking-tight">
            {transport.name}
          </span>
          <span className="text-right text-sm text-muted-foreground">
            {transport.detail}
          </span>
        </li>
      ))}
    </ul>
  );
}

function Tabs() {
  const tabs = [
    { label: "Production", color: "text-danger border-danger/40", name: "billing" },
    { label: "Staging", color: "text-warning border-warning/40", name: "billing" },
    { label: "Local", color: "text-success border-success/40", name: "studio" },
  ];
  const docs = ["invoices", "customers", "Query 1", "invoices structure"];
  return (
    <div className="px-7 pb-7 sm:px-8 sm:pb-8">
      <div className="rounded-2xl border border-white/[.07] bg-[#0a0c10] p-3">
        <div className="flex flex-wrap gap-2">
          {tabs.map((tab, i) => (
            <div
              key={tab.label}
              className={cn(
                "flex h-8 items-center gap-2 rounded-full px-3 text-[13px]",
                i === 0 ? "bg-white/[.08]" : "text-muted-foreground",
              )}
            >
              {tab.name}
              <span
                className={cn(
                  "rounded-full border px-2 text-[11px] leading-[18px]",
                  tab.color,
                )}
              >
                {tab.label}
              </span>
            </div>
          ))}
        </div>
        <div className="mt-3 flex gap-1.5 overflow-hidden rounded-xl border border-white/[.06] bg-[#07090c] p-1.5">
          {docs.map((doc, i) => (
            <div
              key={doc}
              className={cn(
                "flex h-8 shrink-0 items-center rounded-full px-3 text-[13px]",
                i === 2
                  ? "bg-accent text-[#82aaff]"
                  : "text-muted-foreground",
              )}
            >
              {doc}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function Filters() {
  const rows = [
    ["status", "=", "active"],
    ["budget", ">", "20000"],
  ];
  return (
    <div className="space-y-2 px-7 pb-7 sm:px-8 sm:pb-8">
      {rows.map(([column, op, value]) => (
        <div key={column} className="grid grid-cols-[1fr_52px_1fr] gap-2">
          {[column, op, value].map((cell, i) => (
            <div
              key={i}
              className={cn(
                "flex h-9 items-center rounded-full border border-white/[.08] bg-[#0a0c10] px-3.5 text-sm",
                i === 1 && "justify-center font-mono text-muted-foreground",
              )}
            >
              {cell}
            </div>
          ))}
        </div>
      ))}
      <p className="pt-2 font-mono text-[12px] text-muted-foreground">
        <span className="text-sql-keyword">WHERE</span> status ={" "}
        <span className="text-sql-parameter">$1</span>{" "}
        <span className="text-sql-keyword">AND</span> budget &gt;{" "}
        <span className="text-sql-parameter">$2</span>
      </p>
    </div>
  );
}

function Inspector() {
  return (
    <div
      role="img"
      aria-label="DBX row inspector showing every field of the selected row"
      className="relative h-60 overflow-hidden border-t border-white/[.06] bg-[url(/workbench.png)] bg-[length:420%_auto] bg-[position:99%_19%] bg-no-repeat sm:h-64"
    >
      <div className="absolute inset-0 bg-gradient-to-t from-card via-transparent to-transparent" />
    </div>
  );
}

function Transfer() {
  const formats = ["SQL", "CSV", "TSV", ".gz"];
  return (
    <div className="flex flex-wrap gap-2 px-7 pb-7 sm:px-8 sm:pb-8">
      {formats.map((format) => (
        <span
          key={format}
          className="flex h-9 items-center rounded-full border border-white/[.08] bg-[#0a0c10] px-4 font-mono text-sm"
        >
          {format}
        </span>
      ))}
    </div>
  );
}

function Redis() {
  return (
    <div className="px-7 pb-7 font-mono text-[13px] leading-6 sm:px-8 sm:pb-8">
      <div className="rounded-2xl border border-white/[.07] bg-[#0a0c10] p-4">
        <p>
          <span className="text-muted-foreground">›</span>{" "}
          <span className="text-sql-keyword">HGETALL</span>{" "}
          <span className="text-sql-string">session:4f2a</span>
        </p>
        <p className="text-muted-foreground">1) &quot;user_id&quot;</p>
        <p className="text-muted-foreground">2) &quot;1042&quot;</p>
        <p>
          <span className="text-muted-foreground">›</span>{" "}
          <span className="text-sql-keyword">TTL</span>{" "}
          <span className="text-sql-string">session:4f2a</span>
        </p>
        <p className="text-sql-number">(integer) 3600</p>
      </div>
    </div>
  );
}

export function Features() {
  return (
    <section id="features" className="mx-auto max-w-6xl px-6 pb-32">
      <div className="mb-14 max-w-2xl">
        <h2 className="text-[clamp(2.25rem,5vw,3.75rem)] leading-[1] font-medium tracking-[-0.04em]">
          Built for everyday database work
        </h2>
      </div>
      <ScrollReveal className="grid gap-4 md:grid-cols-6">
        <Tile
          className="md:col-span-3"
          title="Reach it however you can"
          body="Paste a connection URL or fill in the details. Tunnel through a bastion, use a local socket, or fetch a short-lived token from your cloud CLI."
        >
          <Transports />
        </Tile>
        <Tile
          className="md:col-span-3"
          title="Keep everything open"
          body="Connect to several databases at once. Every connection gets its own tables, queries, and structure tabs."
        >
          <Tabs />
        </Tile>
        <Tile
          className="md:col-span-2"
          title="Filters, not WHERE clauses"
          body="Stack structured filters on any table. They run as bound parameters, never string-pasted SQL."
        >
          <Filters />
        </Tile>
        <Tile
          className="md:col-span-2"
          title="Every field at a glance"
          body="Select a row to see all of it. Follow a foreign key straight to the row it points at."
        >
          <Inspector />
        </Tile>
        <Tile
          className="md:col-span-2"
          title="Redis, properly"
          body="Browse keys with incremental SCAN, then drop into a command console."
        >
          <Redis />
        </Tile>
        <Tile
          className="md:col-span-3"
          title="Data in, data out"
          body="Export a table or a whole database with gzip and schema-only options. Import SQL dumps, CSV, and TSV."
        >
          <Transfer />
        </Tile>
        <Tile
          className="md:col-span-3"
          title="Looks like it belongs"
          body="Light, dark, or follow the system. Glass chrome over your desktop, with reduce transparency when you want it."
        >
          <div className="flex gap-2 px-7 pb-7 sm:px-8 sm:pb-8">
            {["System", "Light", "Dark"].map((mode, i) => (
              <span
                key={mode}
                className={cn(
                  "flex h-9 items-center rounded-full px-4 text-sm",
                  i === 0
                    ? "bg-primary text-white"
                    : "border border-white/[.08] bg-[#0a0c10] text-muted-foreground",
                )}
              >
                {mode}
              </span>
            ))}
          </div>
        </Tile>
      </ScrollReveal>
    </section>
  );
}
