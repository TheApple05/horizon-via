# Horizon Via

Experimental cross-version multiplayer for **Horizon 26.3**, distributed as a separate native Horizon mod. Includes automatic server detection and a searchable version picker. This is an independent Rust implementation, unaffiliated with ViaVersion or ViaFabricPlus.

## Install

Download [horizon-via-0.1.0.hnmod](https://github.com/TheApple05/horizon-via/releases/download/v0.1.0/horizon-via-0.1.0.hnmod), then import it through the Horizon launcher's native mod workflow. Native mods compile a game for the instance where they are installed. They are not Java `.jar` files and do not run in Fabric or Forge.

For the command line, extract [horizon-via-0.1.0-source.zip](https://github.com/TheApple05/horizon-via/releases/download/v0.1.0/horizon-via-0.1.0-source.zip), then run the official launcher from the extracted folder:

```text
horizon-launcher native-build . --instance "Horizon Via" --profile performance --trust-native-code
```

Install and prepare an official **Horizon 26.3** version in the launcher first, then create/select that instance and run the command. Native builds require an already prepared base game. The launcher handles its Rust toolchain. You need your own Minecraft Java Edition account and resources. The first native build downloads and compiles dependencies; later builds reuse them.

Open **Multiplayer → Horizon Via** to choose a target version. **Auto Detect** reads the server's advertised protocol. Installing the mod enables its experimental adapters; set `HORIZON_OLDER_SERVERS=0` to turn them off for a process. Selection lasts for the current game session.

## Compatibility

The mod pins official Horizon source commit `757ce1bd14f28647e503030d28744a8034b1f3b0` and official horizon-mixins commit `e8cd0aa38cffef49cb2e088ef716baa107148c02`. It builds from the official game source, without a custom Horizon fork. Do not select “Build anyway” with a different source revision: this package deliberately refuses it.

Added experimental target slices: **1.20.5–1.20.6, 1.21–1.21.1, 1.21.2–1.21.3, 1.21.4, 1.21.5, 1.21.6, 1.21.7–1.21.8**. The pinned game already supplies adapters for **1.21.9–1.21.10, 1.21.11, 26.1–26.1.2 and 26.2**, plus its native 26.3 path. An adapter being listed is not a claim that every server or game behavior is verified.

Online authentication across all targets, modified servers, full playthroughs and complete old movement/combat behavior remain unverified. Older releases, snapshots, Classic/Alpha/Beta and Bedrock are unsupported. Some custom item/recipe/registry layouts remain unsupported. This is an experimental release, not complete ViaFabricPlus parity.

## How this native mod builds

Horizon's normal mixin API cannot add all the enum variants and protocol layouts this feature needs. This bundle therefore uses the official native mod crate mechanism: a small Rust **build script** applies a source overlay inside the launcher's disposable staged workspace. Its dependency on `horizon-core` ensures it runs before the affected game libraries compile, including the embedded translations.

The script checks the stage marker, official revision, crate location, permitted destination paths, payload hashes and every expected original file **before writing anything**. It refuses Git checkouts, links and conflicting edits. Only the staged copy is changed; the official source cache and your original checkout are untouched. Modified source files are tracked by Cargo so restoring a stage causes the overlay to run again. Removing the mod and rebuilding restores the original source.

This uses internal source files, not stable hooks. Other mods that change the same files can conflict; the build stops instead of silently replacing their changes. Registry acceptance of this source-overlay technique requires maintainer review. The artifact can be distributed independently; it is not already listed in Horizon's registry.

## Build the distribution

Python 3.11+ is only needed by maintainers to create the source archives:

```text
python tools/package.py
```

This creates the `.hnmod`, source ZIP and `SHA256SUMS.txt` in `dist/`. Players use the launcher's Rust build and do not need Python. See [PROVENANCE.md](NOTICE-PROVENANCE.md) for source origins and [CHANGELOG.md](README-VALIDATION.md) for package validation and limitations.

MIT licensed. Includes original Horizon source under its existing MIT notice. No game jars, Minecraft assets, prepared resources, credentials, worlds or compiled game binaries are included.
