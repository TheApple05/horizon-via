# Source provenance

Horizon Via is an independent native compatibility extension, not an official ViaVersion product.

- Base game: Horizon at `757ce1bd14f28647e503030d28744a8034b1f3b0` (official horizon-rust/horizon, MIT).
- Compatibility source snapshot: TheApple05/horizon at `01166b25ae1e1ad71d32cf76ffaa5b485e8abcba` (MIT).
- Official horizon-mixins at `e8cd0aa38cffef49cb2e088ef716baa107148c02` (MIT, levzzz).
- Generated protocol and registry maps were derived from independently inspected vanilla reports.
- The original project inspected ViaFabricPlus README, LICENSE, AGENTS.md and build.gradle.kts metadata; no ViaFabricPlus or ViaVersion protocol/gameplay implementation was read or copied.
- Selector code was independently implemented from a published screenshot and user documentation. No original mod assets are bundled.
- Source inherited from Horizon keeps its MIT notice in this distribution. Dependencies retain their licenses.

The archive contains source only. It excludes game jars, assets, prepared resources, account/session data, personal launch integrations, worlds, logs and compiled game binaries.

The payload index in `mixins/horizon_via/source_patch/data/overlay.json` records every replaced or added file and both source hashes. Existing files are accepted only with the original or this mod's exact LF-normalized hash; unexpected changes stop the build before writes.
