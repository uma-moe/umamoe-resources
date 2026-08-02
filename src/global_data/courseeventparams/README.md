# Global Course Event Overrides

These decoded source assets override individual rows in the bundled JP course
event set when Global race behavior differs. JP remains the complete fallback;
an override is added only after extracting the matching current Global client
asset and validating the generated simulator resource.

## Course 10501

- Global Steam app: `3224770`
- Global build: `24140954`
- Extraction date: `2026-07-29`
- Logical asset: `race/courseeventparam/10501/pfb_prm_race10501`
- Bundle hash: `WPMTBUC54ZILJPDC2DNUHUBR3CGKZR77`

The Global asset ends corner 4 at `890 m` and starts the following straight at
`890 m`; the corresponding JP asset uses `900 m`. This affects the number of
ten-metre all-corner activation windows that can be sampled during skill
construction and therefore affects the shared race RNG cursor.

Only the decoded JSON source needed by the generator is committed. Original
bundles, generated resources, databases, decryption material, and compiled
artifacts are intentionally excluded.
