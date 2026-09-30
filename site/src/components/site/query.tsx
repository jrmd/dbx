import Image from "next/image";
import { ScrollReveal } from "@/components/jez-ui/ui/scroll-reveal";

const shortcuts = [
  { keys: ["⌘", "↵"], label: "Run the statement under the caret" },
  { keys: ["⌘", "⇧", "↵"], label: "Run the whole document" },
  { keys: ["Esc"], label: "Cancel a running query" },
];

export function Query() {
  return (
    <section className="border-y border-white/[.06] bg-[#0a0c10] py-28 sm:py-36">
      <div className="mx-auto grid max-w-6xl items-center gap-14 px-6 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <div>
          <h2 className="text-[clamp(2.25rem,5vw,3.75rem)] leading-[1] font-medium tracking-[-0.04em]">
            A proper SQL editor
          </h2>
          <p className="mt-6 max-w-md text-lg leading-relaxed text-muted-foreground">
            Syntax highlighting, schema-aware completion, and a result grid
            that stays put while the next query runs. History is kept per
            connection, and loading it never runs anything.
          </p>
          <ul className="mt-10 space-y-3">
            {shortcuts.map((shortcut) => (
              <li key={shortcut.label} className="flex items-center gap-4">
                <span className="flex w-24 shrink-0 gap-1">
                  {shortcut.keys.map((key) => (
                    <kbd
                      key={key}
                      className="flex h-7 min-w-7 items-center justify-center rounded-md border border-white/10 bg-white/[.05] px-1.5 font-sans text-[13px] shadow-[inset_0_-1px_0_#ffffff14]"
                    >
                      {key}
                    </kbd>
                  ))}
                </span>
                <span className="text-muted-foreground">{shortcut.label}</span>
              </li>
            ))}
          </ul>
        </div>
        <ScrollReveal>
          <div className="rounded-[20px] border border-white/10 bg-white/[.03] p-2 shadow-[0_30px_100px_-30px_#2563eb55]">
            <Image
              src="/query.png"
              alt="DBX SQL editor running a join over projects and teams, with eight result rows"
              width={2304}
              height={1472}
              sizes="(min-width: 1024px) 660px, 100vw"
              className="rounded-[12px]"
            />
          </div>
        </ScrollReveal>
      </div>
    </section>
  );
}
