import { ScrollReveal } from "@/components/jez-ui/ui/scroll-reveal";

const steps = [
  { label: "Your passphrase", detail: "Never written to disk" },
  { label: "Argon2id", detail: "Memory-hard key derivation" },
  { label: "XChaCha20-Poly1305", detail: "Authenticated encryption" },
  { label: "DBX Vault", detail: "Sealed credentials, on your machine" },
];

export function Vault() {
  return (
    <section id="vault" className="relative overflow-hidden py-28 sm:py-36">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_50%_60%_at_80%_50%,#2563eb22,transparent_70%)]"
      />
      <div className="relative mx-auto grid max-w-6xl items-center gap-14 px-6 lg:grid-cols-2">
        <div>
          <h2 className="text-[clamp(2.25rem,5vw,3.75rem)] leading-[1] font-medium tracking-[-0.04em]">
            Passwords stay encrypted
          </h2>
          <p className="mt-6 max-w-md text-lg leading-relaxed text-muted-foreground">
            Saved credentials live in an encrypted vault that DBX owns. Your
            connection list stays password-free, and the vault never leaves your
            machine.
          </p>
          <p className="mt-4 max-w-md leading-relaxed text-muted-foreground">
            Prefer not to type the passphrase every launch? Device unlock keeps
            the derived key in macOS Keychain or Secret Service. The passphrase
            itself is never stored.
          </p>
        </div>
        <ScrollReveal>
          <ol className="relative space-y-3">
            {steps.map((step, i) => (
              <li
                key={step.label}
                className="flex items-center justify-between gap-4 rounded-2xl border border-white/[.07] bg-card/80 px-6 py-5 backdrop-blur"
              >
                <span className="flex items-center gap-4">
                  <span className="font-mono text-sm text-muted-foreground">
                    0{i + 1}
                  </span>
                  <span className="font-mono text-[15px]">{step.label}</span>
                </span>
                <span className="text-right text-sm text-muted-foreground">
                  {step.detail}
                </span>
              </li>
            ))}
          </ol>
        </ScrollReveal>
      </div>
    </section>
  );
}
