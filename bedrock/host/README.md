# Unreleased host integration inputs

`registry.rs` is the host registry adapter. `streaming-unlit.patch` adds generic
handling for server columns without usable lighting, with CPU regressions, against
Horizon revision `d8edfbcd0f70aac5b41bab12b8bbf9cc3626de1f`. These are developer
integration inputs. The Bedrock adapter, session and host world are not connected.

From the mod repository root, use Python 3.10 or newer:

```sh
python tools/bedrock/prepare_source_overlay.py --pristine /path/to/exact-d8-export --out /path/to/new-output
```

The export must contain the matching `.horizon-revision` marker. Use ordinary,
case-exact paths; the output directory must be absent and its parent must exist.
The tool validates all 125 legacy payloads, checks replacement preimages after
CRLF-to-LF normalization, and requires new targets to be absent. It preserves those
payload bytes, applies only the reviewed addition-only streaming patch, and records
the actual raw pristine hashes for the SDK.

The output contains 126 full-file payloads and a schema-1 index under
`data/source-overlay`, plus a `source-overlay.toml` snippet for the developer's
`horizon-mixin.toml`. Review and verify the prepared inputs through the SDK before
using them in an integration build.

This tool prepares author inputs. Full Git provenance and execution authority
remain separate; the SDK independently verifies the complete stage. Runtime
building, `.hnmod` installation and a connected Bedrock world remain outside this
tool. No host integration release or gameplay compatibility is implied.
