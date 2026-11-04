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

The optional `nethernet` feature adds settings and offline Login glue over an
already admitted caller-owned transport. It retains the same connection and
proof key, builds metadata after compression settings, bounds input and pack
budgets, disables blob-cache claims, and retires the same connection on setup
failure. Callers must bind transport admission to that same key. This layer
does not discover/connect to servers, acquire credentials, follow transfers,
download resource packs, initialize a player or translate gameplay.

`storage` decodes bounded version-9 network subchunks and full-height biome
storages, preserving separate layers and following-payload offsets. It admits
up to eight layers, 64 sections and 4096 palette entries per storage. Older
storage versions, disk palettes, larger palettes/layer counts, layer composition
and block-entity tails remain outside this decoder. Application receive limits
also cap expanded collections at 4096 elements and individual byte fields at 8 MiB;
larger server data requires a reviewed per-field budget.

Storage format adapted from the MIT implementations at
[Dragonfly 51dd9c1](https://github.com/df-mc/dragonfly/tree/51dd9c1d615f17b70bbeef6ab2cbcee0545e66c0/server/world/chunk)
and [Gophertunnel 0e179fc](https://github.com/Sandertv/gophertunnel/blob/0e179fc4bb082c13e946c7d81ab1cf9f623d942c/minecraft/protocol/varint.go).
The latter's protocol declaration is 2225/1.26.60. Their full MIT notices are
retained. The developer-only `check_storage_oracle.py` authenticates selected
existing reference sources, runs their original palette/packing/varint methods
offline with synthetic IDs, and compares all 36,864 cells across nine widths
against the Rust decoder. It imports no world registry or game assets.

Pinned Cinnabar-authored libraries have MIT grants. Their notices and the
separate CC0 data dedication are retained in `notices/`. Imported protocol
documents, schemas, captures and mixed game-data catalogs are not relicensed by
those grants. The base Cinnabar application's PolyForm implementation is not
used. All-platform transitive distribution-license review remains pending.

CPU-only developer checks, using a separate target directory:

```powershell
$env:CARGO_TARGET_DIR = '<private isolated build directory>'
cargo test --locked --manifest-path bedrock/Cargo.toml -j 2 --lib
cargo check --locked --manifest-path bedrock/Cargo.toml -j 2 --features nethernet --lib
cargo test --locked --manifest-path bedrock/Cargo.toml -j 2 --features nethernet --test nethernet_login -- --test-threads=1
cargo run --locked --manifest-path bedrock/Cargo.toml -j 2 --example check_native_facts -- '<extracted native-facts.json>'
python -m unittest discover -s tools/bedrock -p test_native_facts.py
```

The signed loopback tests use ephemeral synthetic identities and local listeners.
They check offer and packless Login boundaries, FIFO preservation, timeout and
cancellation cleanup. These are parser/library/fixture checks. Owned-server interoperability,
resource-packs/startup, chunk/light/biome rendering, movement, entities,
inventory, dimensions, reconnect and account-backed joining still require
implementation and integration tests before any support claim or mod release.
