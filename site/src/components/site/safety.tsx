import { ScrollReveal } from "@/components/jez-ui/ui/scroll-reveal";
import { cn } from "@/lib/utils";

const points = [
  {
    title: "Updates find rows by primary key",
    body: "Edits and deletes target exactly one row. No key, no guesswork.",
  },
  {
    title: "Value, NULL, or Default",
    body: "Every field says what you mean. An empty string is never quietly a NULL.",
  },
  {
    title: "Destructive means confirmed",
    body: "Truncate, drop, and destructive SQL stop and ask before they run.",
  },
  {
    title: "Your draft survives errors",
    body: "When the database rejects a change, you see why, and your edits stay put.",
  },
];

const environments = [
  { label: "Production", className: "border-danger/50 text-danger bg-danger/10" },
  { label: "Staging", className: "border-warning/50 text-warning bg-warning/10" },
  { label: "Develop", className: "border-primary/60 text-[#82aaff] bg-primary/10" },
  { label: "Local", className: "border-success/50 text-success bg-success/10" },
];

export function Safety() {
  return (
    <section id="safety" className="mx-auto max-w-6xl px-6 py-28 sm:py-36">
      <div className="grid gap-14 lg:grid-cols-2">
        <div>
          <h2 className="text-[clamp(2.25rem,5vw,3.75rem)] leading-[1] font-medium tracking-[-0.04em]">
            Destructive changes ask first
          </h2>
          <p className="mt-6 max-w-md text-lg leading-relaxed text-muted-foreground">
            Label every connection with its environment. Production is red
            everywhere you see it, so you always know what you&apos;re about
            to change.
          </p>
          <div className="mt-10 flex flex-wrap gap-2">
            {environments.map((env) => (
              <span
                key={env.label}
                className={cn(
                  "flex h-9 items-center rounded-full border px-4 text-sm font-medium",
                  env.className,
                )}
              >
                {env.label}
              </span>
            ))}
          </div>
        </div>
        <ScrollReveal className="grid gap-px overflow-hidden rounded-[20px] border border-white/[.07] bg-white/[.07] sm:grid-cols-2">
          {points.map((point) => (
            <div key={point.title} className="bg-card p-7">
              <h3 className="font-medium tracking-tight">{point.title}</h3>
              <p className="mt-2 leading-relaxed text-muted-foreground">
                {point.body}
              </p>
            </div>
          ))}
        </ScrollReveal>
      </div>
    </section>
  );
}
