# Native identity snapshot

`native-facts.json` contains versioned identity, typed state and measurement
facts extracted by `tools/bedrock/prepare_native_facts.py` from the CC0 native
dataset at `didntpot/bedrock-data@1e041bf547b7c12e94d2b998f68d5ed146975742`.
It targets Bedrock 1.26.60, protocol 2225. Its SHA-256 is
`1982736a2ed020c718c9f53cfe1d03f173b8e2142b8287d3d04616a78e821e1a`.
The full dedication is retained in `../notices/native-facts-LICENSE-CC0.txt`.

This snapshot contains no textures, models, protocol schemas or captured world
payloads. Raw light and item measurements do not define host gameplay behavior.
The loader authenticates the complete snapshot before interpreting it.
