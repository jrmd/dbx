# DBX accessibility patch

This is the UI crate from `longbridge/gpui-component` commit
`c6ffd3e166abb43bff8845f0aa61711adb128dcf` (version 0.5.2).
The upstream Apache 2.0 license is retained in `LICENSE-APACHE`.
`gpui-base`, macros and assets remain Git dependencies resolved by DBX's lockfile.

The root Cargo patch selects this source. Its manifest expands the original
workspace dependency settings without changing their requirements or features.
The README and two compile-time test fixtures are retained from the same commit;
the fixture include paths are adjusted for the retained crate layout.

Production changes are limited to `src/select.rs`, `src/list/list.rs`,
`src/list/delegate.rs`, `src/searchable_list/adapter.rs`, and
`src/input/input.rs`, `src/table/state.rs`, and `src/theme/mod.rs`. They propagate select/list/search names; name and expose
selected options; provide guarded accessible focus/activation; announce the
searchable list cursor and empty results; preserve non-searchable active
descendants; skip disabled options; invalidate option identities when search or
items change; refresh deferred popup trees after native actions; and share
accessibility focus with the real input editor. Native select activation uses
the live state transition, avoiding dispatch through an old focus path.

Table cell wrappers and header wrappers use column keys, and a row's horizontal
list uses a constant ID scoped inside the delegate's row element. This preserves
real element/accessibility identities and scoped state when stable rows sort or
columns move. The rendered regression checks fixed and scrolling cells plus
headers. Delegates retain responsibility for unique stable row element IDs.

Active cell overlays draw a two-pixel focus border without a fill. The active
row supplies the selection background, so opaque custom theme colours cannot
paint over cell values (for both fixed and scrolling columns).

The theme has an optional application-owned input-fill override. DBX uses an
opaque canvas fill so stronger input edges do not also brighten the dark input
background and reduce placeholder contrast. The upstream behavior remains the
default when the override is absent; its regression checks both paths.

`patches/gpui-component-accessibility.patch` at the repository root records the
source delta, including regression tests and the fixture-path adjustments.
Do not replace this directory during a dependency update without reapplying or
upstreaming those changes. Preserve the license, update the fixture/source
provenance, and verify native trees/actions alongside the tests. DBX's select
adapter still checks the unchanged base component's concrete root contract.

Run the component regressions with:

```sh
rtk cargo test --manifest-path vendor/gpui-component/Cargo.toml --target-dir target --locked --offline --lib -- --test-threads=1
rtk cargo clippy --manifest-path vendor/gpui-component/Cargo.toml --target-dir target --locked --offline --lib -- -D warnings
```

The retained component Cargo.lock pins its standalone tests to the same GPUI and
base source revisions as DBX. Its test-only dependencies differ from the app's
lockfile. DBX itself continues to build and test with the repository lockfile.
