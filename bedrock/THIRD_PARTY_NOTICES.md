# Bedrock foundation references

The original Rust integration uses the Cinnabar-authored MIT libraries pinned
in Cargo.toml. The extracted networking/client/packs grant and retained notices
are in `notices/bedrock-net-*`, `notices/bedrock-client-LICENSE-AXOLOTL.txt` and
`notices/protocolgen-*`. Those grants do not relicense imported protocol schemas,
game-data catalogs, captures or the PolyForm base application.

`src/storage.rs` adapts network storage algorithms from Dragonfly,
Copyright (c) 2019 Dragonfly Tech, MIT, commit
`51dd9c1d615f17b70bbeef6ab2cbcee0545e66c0`, selected files under
`server/world/chunk/`; and Gophertunnel, Copyright (c) 2019 Sandertv, MIT,
commit `0e179fc4bb082c13e946c7d81ab1cf9f623d942c`,
`minecraft/protocol/varint.go`. The full respective MIT licenses are retained in
`notices/dragonfly-LICENSE-MIT.txt` and `notices/gophertunnel-LICENSE-MIT.txt`.
The adaptation adds checked Rust indexing, bounded allocation and explicit
payload consumption. The developer oracle authenticates these sources before
using their original Go primitives with synthetic values only.

Native identity/state/light facts derive only from the separately CC0-dedicated
didntpot/bedrock-data snapshot at
`1e041bf547b7c12e94d2b998f68d5ed146975742`, release/1.26.60.
Its full dedication is retained in `notices/native-facts-LICENSE-CC0.txt`.
The extractor authenticates all inputs and writes separate provenance. No
geometry, mixed-input catalogs, game assets or raw captured worlds are included.

Dependencies retain their own licenses. This prototype is not a distribution;
the complete enabled-target license/notice audit remains a release gate.
