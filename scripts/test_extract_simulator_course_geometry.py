import importlib.util
import struct
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("extract_simulator_course_geometry.py")
SPEC = importlib.util.spec_from_file_location("extract_course_geometry", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class CourseGeometryExtractorTests(unittest.TestCase):
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
