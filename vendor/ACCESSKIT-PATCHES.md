# DBX's local AccessKit AT-SPI extension

DBX patches the exact previously locked registry releases:

- `accesskit_atspi_common` 0.18.1, registry checksum
  `1e8c61bee90b42a772d39d06a740207dc71a4e780004ace1db8d99fb1baaa954`.
- `accesskit_unix` 0.21.1, registry checksum
  `b016ca8db0ea0ea2ceff29a9d6240391492d960716aa471967c00e8cc8cb197c`.

Both packages record upstream commit
`f40dfc01a0c0e76de535969f82fb35e19513737d` in `.cargo_vcs_info.json`.
The package manifests are Cargo's published normalized manifests. Sources,
README, changelog and VCS metadata are retained. Registry bookkeeping,
package lockfiles and redundant original manifests are omitted. MIT and Apache
licenses are included from that upstream commit; the common adapter also includes
the Chromium license referenced by its existing derived source files.

The workspace's `[patch.crates-io]` selects both packages without changing
AccessKit, its consumer version, GPUI or other registry resolutions. Both adapter
packages are excluded from workspace membership but remain dependencies.

## Changes

The common adapter advertises `Table` on Table/Grid/TreeGrid nodes and
`TableCell` on positioned cells/headers with a containing table. Queries resolve
the current consumer tree under its read lock and expose actual cells, declared
or inferred dimensions, zero-based positions, spans, row/column headers and
selection. Hidden subtrees and nested table contents are excluded. Embedded cell
controls are not counted as extra cells. Invalid coordinates, overflow and stale
nodes are checked. Selected-axis arrays have a bounded allocation guard.

Table queries share an immutable snapshot under the consumer tree read lock.
Each context owns its cache. All semantic writes and host-focus changes pass
through `Context::write_tree`, which acquires the tree write lock and invalidates
the cache before returning it. The tree is private so updates cannot bypass
invalidation. Sorting, selection, reparenting/hiding and removal therefore do not
reuse old metadata. Regressions cover repeated-query reuse and post-write data.
This internal query cache is separate from the native metadata cache below.

The Unix adapter also implements version 1 of `org.a11y.atspi.Cache` at
`/org/a11y/atspi/cache`. `GetItems` returns current filtered public metadata,
including native object, application and parent references, child indices/counts,
names, descriptions, roles, interfaces and states. It contains no separate text,
value or editing payload. Root endpoints are installed before the application is
announced. Add/remove signals follow interface registration and accessible-node
removal; queued nodes already hidden or removed are not announced.

Bulk snapshots release the app lock before acquiring window tree locks. Filter
results are memoized only inside a single tree read lock and discarded afterward.
This keeps public metadata equal to ordinary getters without retaining stale
focus, selection, parent or visibility information across updates. Version 2 and
its asynchronous Ready contract are not advertised. Protocol and privacy tests
cover the wire signature, metadata parity, updates and hidden/protected nodes.
The cache does not by itself establish fast cold-reader startup.

The Unix adapter registers and unregisters both D-Bus interfaces with the other
node interfaces. Its schema tests check scalar and object-reference signatures,
including the single `(so)` containing-table property. An extra wrapper around
that property makes libatspi's libdbus decoder abort.

`TableCell.GetRowColumnSpan` returns `iiii`, matching GNOME's native C client
and ATK bridge. The current published XML and `atspi-proxies` 0.13 generated
proxy include a leading boolean (`biiii`); a native libatspi 2.60.6 call rejects
that reply. GNOME's current
[client source](https://github.com/GNOME/at-spi2-core/blob/main/atspi/atspi-table-cell.c)
still requests `=>iiii`. DBX deliberately follows the working native contract.
The schema regression asserts four scalar integer outputs. A client generated
from the inconsistent XML may require the same correction.

The legacy child-index methods return only real direct-child indices; nested
row cells return `-1`. Modern coordinate and header queries work with nested
rows. Missing virtualized cells return a null accessible, not a fabricated
object. Caption/summary objects are absent where AccessKit supplies none.
Full-row/column selection changes return false because AccessKit lacks an
atomic whole-axis action; existing cell actions continue through DBX.

Logical cells and headers with Focus/ScrollIntoView actions expose Component
before being painted. Their extents remain invalid until actual geometry is
supplied. Virtualized table items remain Visible for discovery, while Showing
tracks painted bounds. Native checks verify focus/reveal of an unpainted loaded
row and horizontal header without changing their accessible object identities.

This is Linux AT-SPI support. It does not change macOS or Windows adapters or
establish a complete screen reader workflow.

## Reproduce and verify

[`../patches/accesskit-atspi-tables.patch`](../patches/accesskit-atspi-tables.patch)
contains all ten changed/new Rust files. Start with the published packages'
`src` directories as `accesskit_atspi_common/src` and `accesskit_unix/src` beneath
a temporary directory, then apply `patch --batch -p1` from that directory.
The resulting files must match the vendored sources byte for byte. Include the
manifest/metadata/license files listed above when recreating the full packages.

From the DBX repository:

```sh
cargo test -p accesskit_atspi_common -p accesskit_unix --locked --offline
cargo clippy -p accesskit_atspi_common -p accesskit_unix --locked --offline --all-targets -- -D warnings
cargo fmt --manifest-path vendor/accesskit_atspi_common/Cargo.toml --check
cargo fmt --manifest-path vendor/accesskit_unix/Cargo.toml --check
```

Native evidence and remaining scope are recorded in
[`../docs/accessibility-implementation.md`](../docs/accessibility-implementation.md).
