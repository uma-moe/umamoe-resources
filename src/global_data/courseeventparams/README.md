# Global Course Event Parameters

This directory pins the decoded `CourseParamTable` assets used by the current
Global client (`1.34.0`, resource version
`10007550`). The generator overlays all 108
simulator course IDs on the bundled JP set, so a Global build never silently
inherits JP course-event boundaries.

The source inventory is content-addressed by SHA-256:
`9194f6cf260e8391ebf8f5a2f3b12647bf043e905769c0a771ca9f2fc663bc0a`. `manifest.json` records the decoded JSON hash, decrypted
bundle hash, and normalized course-event payload hash for every included course.

These assets describe runtime course geometry events: corners, straights,
slopes, lane-width changes, and the first lane-movement point. They do not
define the selectable Practice Race season labels or translate the season
field carried by a race request.

Only decoded JSON needed by the generator is committed. Original bundles,
generated resources, databases, decryption material, and compiled artifacts
remain excluded.
