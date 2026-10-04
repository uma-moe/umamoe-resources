# Global Course Event Parameters

This directory pins the decoded `CourseParamTable` assets used by the current
Global client (`1.35.2`, resource version
`10008010`). The generator overlays all 119 playable
simulator course IDs on the bundled JP set, so a Global build never silently
inherits JP course-event boundaries.

`manifest.json` records the current race asset inventory's SHA-256, decoded JSON
hashes, decrypted bundle hashes, and current CDN asset identities. The 108
previously bundled event tables were checked against the `10008010` manifest
and are unchanged; the 11 Kawasaki, Funabashi, and Morioka tables are now included.

The current master has 121 rows. Longchamp `11201` and `11202` are unused
placeholders without normal-race path assets; `11201` also has an unfinished
event table and `11202` has none. Generation rejects any other missing course
event table instead of silently producing an incomplete simulator course list.

These assets describe runtime course geometry events: corners, straights,
slopes, lane-width changes, and the first lane-movement point. They do not
define the selectable Practice Race season labels or translate the season
field carried by a race request.

Only decoded JSON needed by the generator is committed. Original bundles,
generated resources, databases, decryption material, and compiled artifacts
remain excluded.
