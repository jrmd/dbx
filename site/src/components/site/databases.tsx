import { ScrollReveal } from "@/components/jez-ui/ui/scroll-reveal";

const groups = [
  {
    title: "Relational",
    databases: [
      { name: "PostgreSQL", detail: "Schemas, sessions, locks, foreign keys" },
      { name: "MySQL", detail: "Databases, structure, sessions, locks" },
      { name: "SQLite", detail: "Open any file on disk" },
      { name: "SQL Server", detail: "Indexes, checks, row edits" },
      { name: "CockroachDB", detail: "PostgreSQL wire, TLS options" },
      { name: "Supabase", detail: "Through the PostgreSQL connector" },
    ],
  },
  {
    title: "Embedded and edge",
    databases: [
      { name: "DuckDB", detail: "Local files or in memory" },
      { name: "Turso", detail: "libSQL over HTTP" },
      { name: "Cloudflare D1", detail: "SQL through the REST API" },
    ],
  },
  {
    title: "Analytics",
    databases: [
      { name: "ClickHouse", detail: "HTTP or TLS, ClickHouse Cloud" },
      { name: "BigQuery", detail: "GoogleSQL, datasets, paged jobs" },
      { name: "Snowflake", detail: "SQL API, warehouses, partitions" },
    ],
  },
  {
    title: "Documents, keys, and streams",
    databases: [
      { name: "MongoDB", detail: "JSON commands, collections" },
      { name: "Redis", detail: "SCAN browsing and a console" },
      { name: "Elasticsearch", detail: "Indices and search requests" },
      { name: "Kafka", detail: "Consume and produce on topics" },
    ],
  },
];

export function Databases() {
  return (
    <section id="databases" className="mx-auto max-w-6xl px-6 pb-32">
      <div className="mb-14 max-w-2xl">
        <h2 className="text-[clamp(2.25rem,5vw,3.75rem)] leading-[1] font-medium tracking-[-0.04em]">
          Works with what you run
        </h2>
        <p className="mt-6 max-w-md text-lg leading-relaxed text-muted-foreground">
          Native connectors, each with its own query language and explorer.
          Same keyboard, same grid, same muscle memory.
        </p>
      </div>
      <ScrollReveal className="grid gap-px overflow-hidden rounded-[20px] border border-white/[.07] bg-white/[.07] md:grid-cols-2">
        {groups.map((group) => (
          <div key={group.title} className="bg-card py-6">
            <h3 className="px-7 pb-2 text-sm text-muted-foreground sm:px-8">
              {group.title}
            </h3>
            <ul>
              {group.databases.map((database) => (
                <li
                  key={database.name}
                  className="flex items-baseline justify-between gap-4 px-7 py-2.5 sm:px-8"
                >
                  <span className="font-medium tracking-tight">
                    {database.name}
                  </span>
                  <span className="text-right text-sm text-muted-foreground">
                    {database.detail}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </ScrollReveal>
    </section>
  );
}
