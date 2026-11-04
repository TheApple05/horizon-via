# Unreleased Bedrock backend foundation

This development workspace is not included in the published ViaHorizon 0.3.0
mod. It targets Bedrock 1.26.60, protocol 2225. It does not yet provide a
playable Horizon connection or advertise Bedrock in the version selector.

The existing native-mod SDK only admits dependency identities already present
in Horizon's lockfile. Keeping Bedrock separately publishable requires a reviewed
dependency-plan extension in the native-mod SDK and launcher, including pinned
Git dependencies, root-patch conflict checks, a validated locked dependency graph
and distribution notices. Current published tooling cannot load this prototype.
No dependency guards are bypassed. Player-facing code remains Rust.

`catalog` authenticates and validates the identity/state/light-only CC0 snapshot
produced by `../tools/bedrock/prepare_native_facts.py`. Block ordinals and full
unsigned hashes are explicit distinct lookup modes. Unknown/custom identities
remain unknown. Raw item stack measurements are not gameplay stack rules.
Geometry, assets and mixed-input catalogs are not imported.
`Catalog::bundled()` loads the same authenticated snapshot from the backend's
own `data/native-facts.json`; vanilla identity lookup needs no private file or
runtime download. The full CC0 dedication and version/hash provenance are
included with it. Host semantic rules and custom server identities remain
separate responsibilities.

The optional `nethernet` feature adds settings and offline Login glue over an
already admitted caller-owned transport. It retains the same connection and
proof key, builds metadata after compression settings, bounds input and pack
budgets, disables blob-cache claims, and retires the same connection on setup
failure. Callers must bind transport admission to that same key. This layer
does not discover/connect to servers, acquire credentials, follow transfers or
translate gameplay.

`startup` consumes that Login boundary with its original absolute deadline.
Nonempty resource-pack offers wait for an explicit caller decision while late
verified key exchanges are serviced. Accepted packs use bounded in-band transfer
or caller-verified preloads; the same original protocol owners retain the bytes,
completion notifications, early stack and final acknowledgement. The raw wire
adapter preserves unknown and unrelated original frames in the existing bounded
FIFO. Archives are handed off as opaque bytes; validation and application remain
with the host. No archive is extracted or applied by this layer.

Network readiness retains the first StartGame and item registry. It permits the
host to read original queued world frames but sends no player-initialization
acknowledgement until the host's presentation future succeeds. That consuming
transition sends once on the original connection. Failed presentation, setup
errors, cancellation and timeouts retire that same owner. These library states
are not a working Horizon world adapter or evidence of rendered gameplay.

`storage` decodes bounded version-9 network subchunks and full-height biome
storages, preserving separate layers and following-payload offsets. It admits
up to eight layers, 64 sections and 4096 palette entries per storage. Older
storage versions, disk palettes, larger palettes/layer counts, layer composition
and block-entity tails remain outside this decoder. Application receive limits
also cap expanded collections at 4096 elements and individual byte fields at 8 MiB;
larger server data requires a reviewed per-field budget.

`chunks` checks a cache-disabled LevelChunk against the active dimension and its
negotiated section bounds, decodes explicit version-9 section Y values into
sparse slots, and reads biomes for the full dimension height. Duplicate sections,
cache metadata, request-mode metadata, oversized payloads and wrong dimensions
are refused. Untransmitted sections remain unknown rather than becoming air.
The biome-update flag and exact border/block-entity tail are retained; the tail
is not validated or applied. This is a packet envelope, not a complete world or
permission to acknowledge presentation. Session ownership, tails, omissions and
registry epochs still require host handling before world insertion.

Storage format adapted from the MIT implementations at
[Dragonfly 51dd9c1](https://github.com/df-mc/dragonfly/tree/51dd9c1d615f17b70bbeef6ab2cbcee0545e66c0/server/world/chunk)
and [Gophertunnel 0e179fc](https://github.com/Sandertv/gophertunnel/blob/0e179fc4bb082c13e946c7d81ab1cf9f623d942c/minecraft/protocol/varint.go).
The latter's protocol declaration is 2225/1.26.60. Their full MIT notices are
retained. The developer-only `check_storage_oracle.py` authenticates selected
existing reference sources, runs their original palette/packing/varint methods
offline with synthetic IDs, and compares all 36,864 cells across nine widths
against the Rust decoder. It imports no world registry or game assets.
The LevelChunk envelope follows the same Dragonfly network decoder and the
pinned MIT [Gophertunnel packet](https://github.com/Sandertv/gophertunnel/blob/0e179fc4bb082c13e946c7d81ab1cf9f623d942c/minecraft/protocol/packet/level_chunk.go).

`translation` converts decoded native Y-fast cells into the host's X-fast order.
Its explicit palette mode resolves unsigned wire identities before a host-owned
semantic mapper sees every ordered layer. Unknown identities and unsupported
layer combinations refuse the complete section. Empty storage requires canonical
native air and an explicit host mapping. Returned host IDs are a mapper assertion,
not native IDs; sections still require local lighting. Concrete block semantics,
biomes, world insertion and presentation are not implemented by this boundary.
The separate contextual mapper receives signed section/local coordinates and
uses no host-result tuple cache, including for empty storage. Missing neighborhood
data can be retried; the caller owns a stable context snapshot and rejects stale
connection/dimension/chunk generations before applying a complete result.

`blocks::TerrainRules` implements a narrow, explicit set of natural ground,
axial log/stone and non-falling source-water rules. It requires exact lookup of
the full resulting name and property set in the host's current registry, admits
only canonical air in
additional storage layers, with an explicit exception for air plus source water
in the ordinary second layer. It rejects unknown properties and unsupported blocks.
It has no default-state or numeric-ID fallback. Source water is only typed
`liquid_depth=0`, for either native water name; other depths, falling water,
lava, waterlogged solids and extra non-air layers remain unsupported.
Grass snow context, leaves, slabs, stairs and other layer combinations still
need semantic rules.
This library is not yet wired to Horizon's actual block registry or world.

The source-water and log-axis rules are checked against Dragonfly's pinned MIT
[water encoder](https://github.com/df-mc/dragonfly/blob/51dd9c1d615f17b70bbeef6ab2cbcee0545e66c0/server/block/water.go)
and [log encoder](https://github.com/df-mc/dragonfly/blob/51dd9c1d615f17b70bbeef6ab2cbcee0545e66c0/server/block/log.go).
The host adapter separately checks exact registry states; source-water validation
also checks Horizon's actual fluid kind, amount and falling flag. This is a
limited state translation, not equivalent fluid simulation or rendered-flow proof.

Pinned Cinnabar-authored libraries have MIT grants. Their notices and the
separate CC0 data dedication are retained in `notices/`. Imported protocol
documents, schemas, captures and mixed game-data catalogs are not relicensed by
those grants. The base Cinnabar application's PolyForm implementation is not
used. The [transitive notice index](notices/transitive/manifest.json) covers all
164 added third-party identities in the author-prepared combined Horizon lock,
including their applicable complete license texts and attributions. Those
package grants are permissive; mixed AND obligations retain their full notices.
This is a package-level notice review, not a full per-file or binary-provenance
audit. The index binds combined lock SHA-256
`705f20b07c680348250d59a026e03a66b49d8ef6d9de565397af46680e04ba87`;
it does not claim to describe every version in the separate development lock.
No compiler, import-library binary, schema, capture or game asset is included
in the legal-only notice bundle.

CPU-only developer checks, using a separate target directory:

```powershell
$env:CARGO_TARGET_DIR = '<private isolated build directory>'
cargo test --locked --manifest-path bedrock/Cargo.toml -j 2 --lib
cargo check --locked --manifest-path bedrock/Cargo.toml -j 2 --features nethernet --lib
cargo test --locked --manifest-path bedrock/Cargo.toml -j 2 --features nethernet --test nethernet_login -- --test-threads=1
cargo test --locked --manifest-path bedrock/Cargo.toml -j 2 --features nethernet --test startup -- --test-threads=1
cargo run --locked --manifest-path bedrock/Cargo.toml -j 2 --example check_native_facts -- '<extracted native-facts.json>'
python -m unittest discover -s tools/bedrock -p test_native_facts.py
```

The signed loopback tests use ephemeral synthetic identities and local listeners.
They check offer and packless Login boundaries; empty and explicitly admitted
opaque pack transfer/preloads; early-stack and unknown-frame FIFO preservation;
late verified key ACK ordering and invalid-first-token rejection; presentation
gating; timeout, cancellation and same-owner cleanup. Opaque synthetic pack bytes
are never treated as valid game assets. These are parser/library/fixture checks.
Owned-server interoperability, real resource-pack application, actual Horizon
startup, chunk/light/biome rendering, movement, entities,
inventory, dimensions, reconnect and account-backed joining still require
implementation and integration tests before any support claim or mod release.
