# Simulator Course Geometry

This document records how the optional simulator course-geometry resources are
reconstructed and published. The checked resource source is the golden input for
consumers; they must not read an installed game cache at runtime.

## Source Contract

`src/global_data/simulator_course_geometry.json.gz` contains canonical JSON in gzip
containing all normal-race `CourseLaneAnim` records available for the current
course master. Each course has 1,001 values for position X/Y/Z and quaternion
rotation X/Y/Z/W, plus its course identity, distance, and source asset path.

The asset identity follows the client formatter:

```text
Race/Course/{track:0000}/pos/
an_pos_race{track:0000}_00_{distance:0000}_{ground-1:00}_{around-1}_0
```

The final selector is `CoursePathType.NormalRace`. Story-race geometry is not
silently substituted. The extractor rejects missing or duplicate assets, a
distance mismatch, a non-finite value, or any column that is not exactly 1,001
values long.

## Local Dependencies

The extraction is an offline maintainer operation. It requires:

- an installed current Global client cache and its encrypted `meta` index;
- the SQLite3MultipleCiphers DLL bundled with a local UmaViewer installation;
- the matching local UmaViewer `Config.json`; and
- Python packages pinned in `scripts/requirements-course-geometry.txt`.

These dependencies, decrypted bundles, the game cache, and generated binaries
must not be committed. Only source code and the canonical gzip source belong
in the repository.

## Refresh

Install the pinned Python dependency:

```powershell
python -m pip install -r scripts/requirements-course-geometry.txt
```

Fetch the current game master before checking coverage:

```powershell
cargo run -- fetch-master --master master.mdb
```

The extractor can read that master directly, so newly released courses are not
hidden by an older generated `simulator_courses.json.gz`. Use the matching
current Global cache, configuration, and asset manifest:

```powershell
python scripts/extract_simulator_course_geometry.py --region global --client-root "$env:USERPROFILE\AppData\LocalLow\Cygames\Umamusume" --sqlite3mc "C:\path\to\UmaViewer_Data\Plugins\x86_64\sqlite3mc_x64.dll" --umaviewer-config "C:\path\to\UmaViewer\Config.json" --master master.mdb --out "src\global_data\simulator_course_geometry.json.gz"
```

Use repeatable `--course-id` arguments only for diagnostics. A release refresh
must omit that filter so the source covers every playable course in the current master.

The `1.35.2:10008010` Global master contains 121 course rows. All 119 playable
courses have event tables and 1,001-point paths, including Kawasaki, Funabashi,
and Morioka. Longchamp `11201` (1000 m) and `11202` (1400 m) are unused rows:
the current game asset manifest has no corresponding normal-race paths;
`11201` has an unfinished event table and `11202` has no event table. They are
excluded, and generation fails if the current master begins referencing them
in races. Longchamp `11203` (2400 m) has complete geometry and is exported.

Run the extractor tests and the resource test suite:

```powershell
python -m unittest scripts/test_extract_simulator_course_geometry.py
cargo fmt --check
cargo test --locked
```

Generate public resources:

```powershell
cargo run -- generate --master master.mdb --out generated-data --write-json
```

The pipeline hashes the artifact filename, schema version, and decompressed
canonical JSON for its resource-version prefix, so the bundled format gets a
new version while a zlib implementation change does not masquerade as game-data
drift. It emits one `simulator_course_geometry.json(.gz)` artifact containing
shared `schema_version` and `master_version` fields and a `courses` array.
Each entry contains `course_id`, `race_track_id`, `course_distance`,
`source_asset`, and the seven position/rotation columns. Entries follow the
current course master's order; future courses in the source are excluded.
The manifest lists only the combined artifact, with no per-course artifacts.

The writer fixes gzip metadata, but different compatible zlib versions may emit
different deflate streams for identical input. Compare the decompressed JSON
before deciding that a refresh changed the golden data.

## Consumer Validation

Consumers must load `simulator_course_geometry.json.gz` and select an entry
from `courses` by `course_id`, replacing per-course filename requests. For
example, browsers decompress the HTTP response automatically:

```javascript
const response = await fetch("/resources/current/simulator_course_geometry.json.gz");
if (!response.ok) throw new Error(`Course geometry request failed: ${response.status}`);
const geometry = await response.json();
const course = geometry.courses.find(entry => entry.course_id === courseId);
```

All current master courses must be found and valid with zero missing, invalid,
or metadata-mismatched entries. Validated geometry is available
to standalone world/lane tools; it does not enable geometry correction in the
canonical race loop. That change still requires proof of the server's exact
ratio-update ordering.

## Finish-line continuation lanes

`src/global_data/simulator_course_lanes.json.gz` retains the existing Global
`course-lanes.json` continuation data: each course's `courseId` and `overrun`
asset path, plus the referenced assets with their original transform samples.
Normal course transforms remain in `simulator_course_geometry.json.gz`.
The reduced continuation source currently contains 121 courses and 25 shared
assets; `source_sha256` identifies the full retained input.

Maintainers can regenerate it without game-cache or third-party dependencies:

```sh
python scripts/export_simulator_course_lanes.py /path/to/course-lanes.json src/global_data/simulator_course_lanes.json.gz
```

The resource generator checks that every exported course has its continuation
asset and publishes `simulator_course_lanes.json.gz` through the normal manifest.
The geometry version hash also covers this source. Consumers validate the
published hash, then compile the continuation samples with the matching normal
geometry. No interpolation, resampling or simulator-host file copy is involved.

The internal resource listener additionally serves the exact master database
used for the export at the manifest's `master.path`. Its uncompressed bytes and
hash must match `master.bytes` and `master.sha256`. Together these five simulator
artifacts form one complete version suitable for automatic startup and refresh.
