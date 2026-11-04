# Source provenance

ViaHorizon is an independent native compatibility extension, not an official ViaVersion product.

Releases 0.1.0 and 0.2.0 were published as Horizon Via. Their source references, release names and license credits remain historical records.

- Base game for 0.2.0 and 0.3.0: Horizon at `d8edfbcd0f70aac5b41bab12b8bbf9cc3626de1f` (official horizon-rust/horizon, MIT).
- Compatibility source: TheApple05/horizon at `7dde5f518c3a24117bdb7b2f28797bef8a06d22d`, merged with the official base above (MIT). Earlier source contributions are historical references; new compatibility work is maintained in this independent mod.
- Version picker and mod installation behavior: this project's 0.1.0 source at `1d905edfb783f63a07f9bba734409481d4efb64a`, retained independently of personal launcher/authentication work.
- Official horizon-mixins at `e8cd0aa38cffef49cb2e088ef716baa107148c02` (MIT, levzzz).
- Generated protocol and registry maps were derived from independently inspected vanilla reports.
- The original project inspected ViaFabricPlus README, LICENSE, AGENTS.md and build.gradle.kts metadata; no ViaFabricPlus or ViaVersion protocol/gameplay implementation was read or copied.
- Selector code was independently implemented from a published screenshot and user documentation. No original mod assets are bundled.
- Source inherited from Horizon keeps its MIT notice in this distribution. Dependencies retain their licenses.

The archive contains source only. It excludes game jars, assets, prepared resources, account/session data, personal launch integrations, worlds, logs and compiled game binaries.

The payload index in `mixins/horizon_via/source_patch/data/overlay.json` records every replaced or added file and both source hashes. Existing files are accepted only with the original or this mod's exact LF-normalized hash; unexpected changes stop the build before writes.

## Java 1.8.9 additions

The protocol 47 implementation is original Rust. Numeric IDs, state descriptors, old lighting properties, packet layouts and synthetic test facts were checked using independently executed official 1.8.9 packet writers/registries and the pinned current reports. Item upgrades use Horizon's existing datafix implementation. No ViaVersion/ViaFabricPlus implementation was read or copied for this adapter.

Official jars, bytecode/decompiled reference output, resources, server saves, live packet captures and local oracle executables are not distributed. Two scalar test-fact datasets are stored as portable ASCII hex; public sign tests construct their synthetic inputs in Rust. All existing Horizon MIT notices and source pins are preserved.
