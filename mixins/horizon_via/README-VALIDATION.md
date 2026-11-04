# Changelog

## 0.1.0 — experimental

- Extracted the native older-server adapters into a separate `.hnmod` source distribution pinned to an official Horizon revision.
- Added the searchable Horizon Via version selector, including shared-protocol release names through 1.20.5/1.20.6.
- Installing the mod enables its experimental connection path. The current-version path remains available; `HORIZON_OLDER_SERVERS=0` disables the adapters.
- Added a guarded, hash-checked source overlay using the official native crate build mechanism. No personal sign-in integration is included.

### Package validation

Validated on Windows with Rust 1.99.1 using source extracted from the native archive and the official pinned Horizon source. These results are from the separate mod, rather than historical tests of the built-in implementation.

- The official `cargo horizon-mod check` accepts the archive as a native mod. The official pinned `horizon-mixins apply` accepts and stages its crate with locked dependencies.
- **271 focused tests passed:** 255 protocol tests, 7 source-installer tests, 8 version/search/navigation tests and 1 legacy-particle regression.
- Workspace formatting and Clippy for the affected protocol/client library and installer library passed with warnings denied.
- Reset all 43 overlaid source files to the pristine state, then rebuilt through Cargo. The installer reapplied every file; all output hashes matched. The official source cache and nine existing personal UI/sign-in files remained unchanged.
- Native offline 1.20.6 and 26.3 server checks passed world rendering, movement, inventory, placing/breaking, dimension travel and reconnects. The older-version check also covered chat, crouching and a server-confirmed chest transfer.
- Two broad loopback probe connections each on 1.20.6, 1.21.8 and 26.3 passed without packet/chunk decode errors; each included dimension changes and recipe unlocks.
- Fresh-profile UI checks passed branding, shared-version searches and observed handshakes 766, 767 and 777. Back preserved the direct address. Unit tests covered Escape/focus behavior. `HORIZON_OLDER_SERVERS=0` hid the picker. The rendered selector was visually inspected.

The normal launcher installation/download flow, Windows GNU compiler, Linux and macOS have not been tested end to end for this release. The build used the same official source transformer and locked Cargo compilation steps with the installed Windows Rust toolchain. Public registry listing requires Horizon's review.

Online authentication across the target matrix, modified servers, full playthroughs, complete older movement/combat behavior and older/custom protocol boundaries remain outside this validation. See the README for supported and unsupported targets.
