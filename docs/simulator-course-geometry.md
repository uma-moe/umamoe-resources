# Simulator Course Geometry

This document records how the optional simulator course-geometry resources are
reconstructed and published. The checked resource source is the golden input for
consumers; they must not read an installed game cache at runtime.

## Source Contract

`src/jp_data/simulator_course_geometry.json.gz` contains canonical JSON in gzip
containing all normal-race `CourseLaneAnim` records available for the current
course master. Each course has 1,001 values for position X/Y/Z and quaternion
rotation X/Y/Z/W, plus its course identity, distance, and source asset path.

The asset identity comes from the JP client decomp:

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

- an installed JP client cache and its encrypted `meta` index;
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

Generate or locate the current `simulator_courses.json.gz`, then run:

```powershell
python scripts/extract_simulator_course_geometry.py --client-root "$env:USERPROFILE\AppData\LocalLow\Cygames\umamusume" --sqlite3mc "C:\path\to\UmaViewer_Data\Plugins\x86_64\sqlite3mc_x64.dll" --umaviewer-config "C:\path\to\UmaViewer\Config.json" --courses "generated-data\<version>\simulator_courses.json.gz" --out "src\jp_data\simulator_course_geometry.json.gz"
```

Use repeatable `--course-id` arguments only for diagnostics. A release refresh
must omit that filter so the source covers every course in the course master.

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

The pipeline hashes the decompressed canonical JSON for its resource-version
prefix, so a zlib implementation change does not masquerade as game-data drift.
It emits one flat `simulator_course_geometry_<course_id>.json(.gz)` artifact
per course. This keeps ordinary simulator and skill-viewer requests independent
of the roughly 6.35 MiB all-course compressed source.

The writer fixes gzip metadata, but different compatible zlib versions may emit
different deflate streams for identical input. Compare the decompressed JSON
before deciding that a refresh changed the golden data.

## Consumer Validation

Point the simulator at the generated version directory and require a complete
audit:

```powershell
$env:UMAMOE_SIM_RESOURCE_DIR = "C:\path\to\generated-data\<version>"
cargo run -p umamoe-sim-cli -- geometry-audit --require --format json
```

All current master courses must be found and valid with zero missing, invalid,
or metadata-mismatched artifacts. Passing this audit makes geometry available
to standalone world/lane tools; it does not enable geometry correction in the
canonical race loop. That change still requires proof of the server's exact
ratio-update ordering.
