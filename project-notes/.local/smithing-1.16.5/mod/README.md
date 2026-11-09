# ViaHorizon

Experimental cross-version multiplayer for **Horizon 26.3**, distributed as a separate native Horizon mod. Includes automatic server detection and a searchable version picker. This is an independent Rust implementation, unaffiliated with ViaVersion or ViaFabricPlus.

## Install

Prepare an official **Horizon 26.3** instance first, then import the `.hnmod` from this project's releases through the Horizon launcher's native mod workflow. Native mods compile a game for the instance where they are installed. They are not Java `.jar` files and do not run in Fabric or Forge. Use the matching release's archive and source ZIP.

For the command line, extract the matching source ZIP, then run the official launcher from the extracted folder:

```text
horizon-launcher native-build . --instance "ViaHorizon" --profile performance --trust-native-code
```

Install and prepare an official **Horizon 26.3** version in the launcher first, then create/select that instance and run the command. Native builds require an already prepared base game. The launcher handles its Rust toolchain. You need your own Minecraft Java Edition account and resources. The first native build downloads and compiles dependencies; later builds reuse them.

Open **Multiplayer → ViaHorizon** to choose a target version. **Auto Detect** reads the server's advertised protocol. Installing the mod enables its experimental adapters; set `HORIZON_OLDER_SERVERS=0` to turn them off for a process. Selection lasts for the current game session.

## Compatibility

Version 0.4.1 pins official Horizon source commit `d8edfbcd0f70aac5b41bab12b8bbf9cc3626de1f` and official horizon-mixins commit `e8cd0aa38cffef49cb2e088ef716baa107148c02`. It builds from the official game source, without a custom Horizon fork. Do not select “Build anyway” with a different source revision: this package deliberately refuses it.

Experimental target slices: **1.8.9, 1.12.2, 1.16.5, 1.20–1.20.1, 1.20.2, 1.20.3–1.20.4, 1.20.5–1.20.6, 1.21–1.21.1, 1.21.2–1.21.3, 1.21.4, 1.21.5, 1.21.6, 1.21.7–1.21.8**. The pinned game already supplies adapters for **1.21.9–1.21.10, 1.21.11, 26.1–26.1.2 and 26.2**, plus its native 26.3 path. These cover 27 named Java releases across 18 protocol groups, including the native 26.3 group. See the corresponding GitHub release notes for the exact package's validation evidence. An adapter being listed is not a claim that every server or game behavior is verified.

Online authentication across all targets, modified servers, full playthroughs and complete old movement/combat behavior remain unverified. Native legacy item/entity hover presentation and reconstructing some edited potion contents for transmission remain incomplete. Java release protocols in the gaps between the listed targets are not included. Java 1.16.5 smithing upgrades use two inputs and require no template. Other older releases, snapshots, Classic/Alpha/Beta and Bedrock are unsupported. Some custom item/recipe/registry layouts remain unsupported. This is an experimental release, not complete ViaFabricPlus parity.

## Java 1.8.9

The native protocol 47 adapter covers login, world/chunk/light conversion, numeric items and metadata, inventory transactions, chat, entities, effects and dimension changes. It uses Rust throughout; there is no Java proxy helper at runtime.

Development acceptance passed on a real local vanilla 1.8.9 server with the native Windows client: 20 gameplay gates and 7 controlled malformed-input/reconnect gates, with zero normal decoding or untranslated-metadata diagnostics. This covered partial-chunk preservation, exact item variants/NBT, rejected chest transactions, creative pick, movement, block interactions, entities, lighting, Nether travel and reconnect. Source and binary hashes were frozen for those runs. The matching release notes record the separate exact-archive checks.

This is experimental. Complete old movement/combat and vehicle-control parity are unfinished. Footprint and snow-shovel particles are recognized cosmetic omissions, and some edited item/menu paths remain unsupported. Newer items which cannot be represented on the old server are rejected locally and their inventory changes are restored.

## How this native mod builds

Horizon's normal mixin API cannot add all the enum variants and protocol layouts this feature needs. This bundle therefore uses the official native mod crate mechanism: a small Rust **build script** applies a source overlay inside the launcher's disposable staged workspace. Its dependency on `horizon-core` ensures it runs before the affected game libraries compile, including the embedded translations.

The script checks the stage marker, official revision, crate location, permitted destination paths, payload hashes and every expected original file **before writing anything**. It refuses Git checkouts, links and conflicting edits. Only the staged copy is changed; the official source cache and your original checkout are untouched. Modified source files are tracked by Cargo so restoring a stage causes the overlay to run again. Removing the mod and rebuilding restores the original source.

This uses internal source files, not stable hooks. Other mods that change the same files can conflict; the build stops instead of silently replacing their changes. Registry acceptance of this source-overlay technique requires maintainer review. The artifact can be distributed independently; it is not already listed in Horizon's registry.

## Build the distribution

Python 3.11+ is only needed by maintainers to create the source archives:

```text
python tools/package.py
```

This creates the `.hnmod`, source ZIP and `SHA256SUMS.txt` in `dist/`. Players use the launcher's Rust build and do not need Python. See [PROVENANCE.md](PROVENANCE.md) for source origins and [CHANGELOG.md](CHANGELOG.md) for package validation and limitations.

MIT licensed. Includes original Horizon source under its existing MIT notice. No game jars, Minecraft assets, prepared resources, credentials, worlds or compiled game binaries are included.
