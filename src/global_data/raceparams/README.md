# Global Race Parameters

The decoded source asset in this directory is the authoritative Global client
input for simulator-wide serialized race constants.

## Global 10006800

- Global Steam app: `3224770`
- Global build: `24140954`
- Client resource version: `10006800`
- Extraction date: `2026-07-29`
- Logical asset: `Race/RaceMain/ast_race_paramdefine`
- Bundle hash: `GQFJZWZJDWLTBRKLLW5VYVPZSUSQBDM6`
- Asset `paramDefineVersion`: `20180828`
- Decoded source: `10006800.json`

`10006800.json` is the Unity MonoBehaviour type tree after the installed
client's asset wrapper was decoded. Numeric values preserve the serialized
single-precision payload as JSON numbers. The simulator generator selects only
fields used by race mechanics; camera, presentation, audio, and commentary
fields remain in the source tree so future audits can distinguish an absent
field from an omitted extraction.

Original bundles, generated resources, databases, decryption material, and
compiled artifacts are intentionally excluded.
