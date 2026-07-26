#!/usr/bin/env python3
"""Extract source CourseLaneAnim keyframes from an installed JP client."""

from __future__ import annotations

import argparse
import ctypes
import gzip
import json
import math
import struct
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


SQLITE_OK = 0
SQLITE_ROW = 100
SQLITE_DONE = 101
SQLITE_OPEN_READONLY = 1
KEYFRAME_COUNT = 1001
SCHEMA_VERSION = 1
HEADER_SIZE = 256
DB_BASE_KEY_CYCLE = 13


@dataclass(frozen=True)
class AssetRecord:
    name: str
    hash: str
    key: int


class EncryptedAssetIndex:
    def __init__(self, database: Path, sqlite3mc: Path, key: bytes) -> None:
        self._lib = ctypes.CDLL(str(sqlite3mc))
        self._configure_api()
        self._db = ctypes.c_void_p()
        result = self._lib.sqlite3_open_v2(
            str(database).encode(),
            ctypes.byref(self._db),
            SQLITE_OPEN_READONLY,
            None,
        )
        if result != SQLITE_OK:
            raise RuntimeError(f"failed to open encrypted asset index: SQLite error {result}")
        key_buffer = ctypes.create_string_buffer(key)
        result = self._lib.sqlite3_key(self._db, key_buffer, len(key))
        if result != SQLITE_OK:
            self.close()
            raise RuntimeError(f"failed to unlock encrypted asset index: SQLite error {result}")

    def _configure_api(self) -> None:
        pointer = ctypes.c_void_p
        library = self._lib
        library.sqlite3_open_v2.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(pointer),
            ctypes.c_int,
            ctypes.c_char_p,
        ]
        library.sqlite3_open_v2.restype = ctypes.c_int
        library.sqlite3_close.argtypes = [pointer]
        library.sqlite3_close.restype = ctypes.c_int
        library.sqlite3_key.argtypes = [pointer, pointer, ctypes.c_int]
        library.sqlite3_key.restype = ctypes.c_int
        library.sqlite3_prepare_v2.argtypes = [
            pointer,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.POINTER(pointer),
            ctypes.POINTER(ctypes.c_char_p),
        ]
        library.sqlite3_prepare_v2.restype = ctypes.c_int
        library.sqlite3_step.argtypes = [pointer]
        library.sqlite3_step.restype = ctypes.c_int
        library.sqlite3_finalize.argtypes = [pointer]
        library.sqlite3_finalize.restype = ctypes.c_int
        library.sqlite3_column_text.argtypes = [pointer, ctypes.c_int]
        library.sqlite3_column_text.restype = pointer
        library.sqlite3_column_bytes.argtypes = [pointer, ctypes.c_int]
        library.sqlite3_column_bytes.restype = ctypes.c_int
        library.sqlite3_column_int64.argtypes = [pointer, ctypes.c_int]
        library.sqlite3_column_int64.restype = ctypes.c_longlong
        library.sqlite3_errmsg.argtypes = [pointer]
        library.sqlite3_errmsg.restype = ctypes.c_char_p

    def load_lane_assets(self) -> dict[str, AssetRecord]:
        statement = ctypes.c_void_p()
        query = (
            b"SELECT n,h,e FROM a "
            b"WHERE n LIKE 'race/course/%/pos/an_pos_race%' ORDER BY n"
        )
        result = self._lib.sqlite3_prepare_v2(
            self._db, query, -1, ctypes.byref(statement), None
        )
        if result != SQLITE_OK:
            message = self._lib.sqlite3_errmsg(self._db).decode(errors="replace")
            raise RuntimeError(f"failed to query encrypted asset index: {message}")
        records: dict[str, AssetRecord] = {}
        try:
            while True:
                result = self._lib.sqlite3_step(statement)
                if result != SQLITE_ROW:
                    if result != SQLITE_DONE:
                        message = self._lib.sqlite3_errmsg(self._db).decode(errors="replace")
                        raise RuntimeError(f"failed while reading asset index: {message}")
                    break
                name = self._column_text(statement, 0)
                records[name] = AssetRecord(
                    name=name,
                    hash=self._column_text(statement, 1),
                    key=int(self._lib.sqlite3_column_int64(statement, 2)),
                )
        finally:
            self._lib.sqlite3_finalize(statement)
        return records

    def _column_text(self, statement: ctypes.c_void_p, index: int) -> str:
        pointer = self._lib.sqlite3_column_text(statement, index)
        size = self._lib.sqlite3_column_bytes(statement, index)
        return ctypes.string_at(pointer, size).decode()

    def close(self) -> None:
        if self._db:
            self._lib.sqlite3_close(self._db)
            self._db = ctypes.c_void_p()

    def __enter__(self) -> "EncryptedAssetIndex":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-root", required=True, type=Path)
    parser.add_argument("--sqlite3mc", required=True, type=Path)
    parser.add_argument("--umaviewer-config", required=True, type=Path)
    parser.add_argument("--courses", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument(
        "--course-id",
        action="append",
        type=int,
        default=[],
        help="Extract only this course ID; may be repeated.",
    )
    return parser.parse_args()


def load_json(path: Path) -> Any:
    if path.suffix == ".gz":
        with gzip.open(path, "rt", encoding="utf-8") as source:
            return json.load(source)
    with path.open("r", encoding="utf-8") as source:
        return json.load(source)


def derive_database_key(base_key: bytes, database_key: bytes) -> bytes:
    if len(base_key) < DB_BASE_KEY_CYCLE:
        raise ValueError("database base key must contain at least 13 bytes")
    return bytes(
        value ^ base_key[index % DB_BASE_KEY_CYCLE]
        for index, value in enumerate(database_key)
    )


def expand_asset_key(base_key: bytes, asset_key: int) -> bytes:
    key_bytes = struct.pack("<q", asset_key)
    return bytes(value ^ key_byte for value in base_key for key_byte in key_bytes)


def decrypt_asset_bundle(data: bytes, base_key: bytes, asset_key: int) -> bytes:
    if asset_key == 0 or len(data) <= HEADER_SIZE:
        return data
    key = expand_asset_key(base_key, asset_key)
    result = bytearray(data)
    for index in range(HEADER_SIZE, len(result)):
        result[index] ^= key[index % len(key)]
    return bytes(result)


def source_asset_name(course: dict[str, Any]) -> str:
    track = int(course["race_track_id"])
    distance = int(course["distance"])
    ground_path = int(course["surface"]) - 1
    around_path = int(course["course"]) - 1
    if ground_path < 0 or around_path < 0:
        raise ValueError(f"course {course['course_id']} has invalid path enum values")
    return (
        f"race/course/{track:04d}/pos/"
        f"an_pos_race{track:04d}_00_{distance:04d}_{ground_path:02d}_"
        f"{around_path}_0"
    )


def load_geometry(bundle: bytes, expected_name: str) -> dict[str, Any]:
    try:
        import UnityPy
    except ImportError as error:
        raise RuntimeError(
            "UnityPy is required; install scripts/requirements-course-geometry.txt"
        ) from error

    environment = UnityPy.load(bundle)
    expected_object_name = expected_name.rsplit("/", 1)[-1]
    matches: list[dict[str, Any]] = []
    for obj in environment.objects:
        if obj.type.name != "MonoBehaviour":
            continue
        tree = obj.read_typetree()
        if tree.get("m_Name") == expected_object_name and "key" in tree:
            matches.append(tree)
    if len(matches) != 1:
        raise ValueError(
            f"{expected_name} contains {len(matches)} matching CourseLaneAnim objects"
        )
    return matches[0]


def finite_column(values: Iterable[Any], field: str, course_id: int) -> list[float]:
    result = [float(value) for value in values]
    if len(result) != KEYFRAME_COUNT:
        raise ValueError(
            f"course {course_id} {field} has {len(result)} values; expected {KEYFRAME_COUNT}"
        )
    if not all(math.isfinite(value) for value in result):
        raise ValueError(f"course {course_id} {field} contains a non-finite value")
    return result


def build_artifact(
    course: dict[str, Any], source_asset: str, tree: dict[str, Any], master_version: str
) -> dict[str, Any]:
    course_id = int(course["course_id"])
    expected_distance = int(course["distance"])
    if int(tree["Distance"]) != expected_distance:
        raise ValueError(
            f"course {course_id} asset distance {tree['Distance']} != {expected_distance}"
        )
    key = tree["key"]
    rotations = key["rotation"]
    if len(rotations) != KEYFRAME_COUNT:
        raise ValueError(
            f"course {course_id} rotation has {len(rotations)} values; "
            f"expected {KEYFRAME_COUNT}"
        )
    return {
        "schema_version": SCHEMA_VERSION,
        "master_version": master_version,
        "course_id": course_id,
        "race_track_id": int(course["race_track_id"]),
        "course_distance": expected_distance,
        "source_asset": source_asset,
        "position_x": finite_column(key["valueX"], "position_x", course_id),
        "position_y": finite_column(key["valueY"], "position_y", course_id),
        "position_z": finite_column(key["valueZ"], "position_z", course_id),
        "rotation_x": finite_column(
            (rotation["x"] for rotation in rotations), "rotation_x", course_id
        ),
        "rotation_y": finite_column(
            (rotation["y"] for rotation in rotations), "rotation_y", course_id
        ),
        "rotation_z": finite_column(
            (rotation["z"] for rotation in rotations), "rotation_z", course_id
        ),
        "rotation_w": finite_column(
            (rotation["w"] for rotation in rotations), "rotation_w", course_id
        ),
    }


def write_bundle(path: Path, value: Any) -> None:
    encoded = json.dumps(
        value, allow_nan=False, ensure_ascii=True, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.suffix != ".gz":
        path.write_bytes(encoded)
        return
    with path.open("wb") as destination:
        with gzip.GzipFile(fileobj=destination, mode="wb", filename="", mtime=0) as target:
            target.write(encoded)


def main() -> int:
    args = parse_args()
    config = load_json(args.umaviewer_config)
    base_db_key = bytes.fromhex(config["DBBaseKeyText"])
    db_key = derive_database_key(base_db_key, bytes.fromhex(config["DBKeyText"]))
    asset_base_key = bytes.fromhex(config["ABKeyText"])

    courses_document = load_json(args.courses)
    master_version = str(courses_document["master_version"])
    requested = set(args.course_id)
    courses = [
        course
        for course in courses_document["courses"]
        if not requested or int(course["course_id"]) in requested
    ]
    if requested and requested != {int(course["course_id"]) for course in courses}:
        missing = sorted(requested - {int(course["course_id"]) for course in courses})
        raise ValueError(f"requested course IDs are absent from simulator_courses: {missing}")

    meta_path = args.client_root / "meta"
    with EncryptedAssetIndex(meta_path, args.sqlite3mc, db_key) as index:
        assets = index.load_lane_assets()

    artifacts = []
    for position, course in enumerate(courses, start=1):
        source_asset = source_asset_name(course)
        asset = assets.get(source_asset)
        if asset is None:
            raise FileNotFoundError(
                f"course {course['course_id']} source asset is absent: {source_asset}"
            )
        asset_path = args.client_root / "dat" / asset.hash[:2] / asset.hash
        encrypted = asset_path.read_bytes()
        tree = load_geometry(
            decrypt_asset_bundle(encrypted, asset_base_key, asset.key), source_asset
        )
        artifacts.append(build_artifact(course, source_asset, tree, master_version))
        print(
            f"[{position:03d}/{len(courses):03d}] course {course['course_id']} "
            f"<- {source_asset}",
            file=sys.stderr,
        )

    artifacts.sort(key=lambda artifact: artifact["course_id"])
    write_bundle(
        args.out,
        {
            "schema_version": SCHEMA_VERSION,
            "source_master_version": master_version,
            "courses": artifacts,
        },
    )
    print(f"wrote {len(artifacts)} course geometries to {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
