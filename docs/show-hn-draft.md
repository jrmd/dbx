# Show HN draft

**Title:** Show HN: DBX – A free, safety-first database client for Linux and macOS

**Link:** https://dbx.bonusobjective.com

I built DBX because I wanted a native database workbench on Linux with the
safeguards I want when changing data.

Edits are staged and shown as SQL before you commit. Updates check row identity
and whether the original values changed. Protected profiles block writes.
Credentials and recovery drafts live in an encrypted local vault, and recovered
work needs review before it can be applied. AI suggestions stay drafts until
you choose to run them.

It is written in Rust with GPUI, supports Linux x86_64 and Apple Silicon macOS,
and connects to fifteen database engines. Every feature is free under MIT:
there is no paid export tier, tab limit, account or telemetry. Connector depth
varies; the capability matrix documents the boundaries.

DBX is still in active development. I would especially like feedback from
Linux users on keyboard workflows, database imports and the places where the
write review gets in your way or misses something useful.

Source: https://github.com/jrmd/dbx

## Before submitting

Use the release that contains the verified launch fixes. Keep this draft in the
repository until the maintainer chooses to post it. No comparative performance
claims are included; benchmarking was deferred on 19 October 2026.
