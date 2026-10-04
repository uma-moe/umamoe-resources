import importlib.util
import struct
import sqlite3
import sys
import tempfile
import unittest
from contextlib import closing
from pathlib import Path


SCRIPT = Path(__file__).with_name("extract_simulator_course_geometry.py")
SPEC = importlib.util.spec_from_file_location("extract_course_geometry", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class CourseGeometryExtractorTests(unittest.TestCase):
    def test_master_input_includes_new_courses_and_excludes_only_unused_placeholders(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "master.mdb"
            Path(str(path) + ".version").write_text("1.35.2:10008010\n", encoding="utf-8")
            with closing(sqlite3.connect(path)) as connection:
                connection.executescript("""
                    CREATE TABLE race_course_set (id INTEGER, race_track_id INTEGER, distance INTEGER, ground INTEGER, inout INTEGER);
                    CREATE TABLE race (course_set INTEGER);
                    INSERT INTO race_course_set VALUES
                        (11201, 10201, 1000, 1, 1), (11202, 10201, 1400, 1, 1),
                        (11301, 10103, 1400, 2, 1), (11504, 10105, 2000, 2, 1);
                """)
            document = MODULE.load_master_courses(path)
            self.assertEqual(document["master_version"], "1.35.2:10008010")
            self.assertEqual([course["course_id"] for course in document["courses"]], [11301, 11504])
            self.assertEqual(document["courses"][0]["surface"], 2)
            with closing(sqlite3.connect(path)) as connection:
                connection.execute("INSERT INTO race VALUES (11201)")
                connection.commit()
            with self.assertRaisesRegex(ValueError, "placeholders now appear"):
                MODULE.load_master_courses(path)

    def test_source_asset_name_matches_decompiled_formatter(self):
        course = {
            "course_id": 10307,
            "race_track_id": 10003,
            "distance": 2000,
            "surface": 1,
            "course": 3,
        }
        self.assertEqual(
            MODULE.source_asset_name(course),
            "race/course/10003/pos/an_pos_race10003_00_2000_00_2_0",
        )

    def test_database_key_uses_thirteen_byte_base_cycle(self):
        base = bytes(range(16))
        key = bytes(range(32))
        derived = MODULE.derive_database_key(base, key)
        self.assertEqual(derived, bytes(value ^ base[i % 13] for i, value in enumerate(key)))

    def test_database_key_selects_requested_region_without_exposing_key_material(self):
        config = {"DBKeyText": "0011", "GlobalDBKeyText": "aabbcc"}
        self.assertEqual(MODULE.configured_database_key(config, "jp"), bytes.fromhex("0011"))
        self.assertEqual(
            MODULE.configured_database_key(config, "global"), bytes.fromhex("aabbcc")
        )

    def test_asset_key_matches_umaviewer_expansion_order(self):
        base = bytes((0x53, 0x2B))
        asset_key = 0x0102030405060708
        key_bytes = struct.pack("<q", asset_key)
        self.assertEqual(
            MODULE.expand_asset_key(base, asset_key),
            bytes([value ^ item for value in base for item in key_bytes]),
        )


if __name__ == "__main__":
    unittest.main()
