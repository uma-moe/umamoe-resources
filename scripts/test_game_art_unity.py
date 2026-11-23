"""Run in the service image, which has UnityPy:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_unity
"""
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import game_art_uma as uma  # noqa: E402
import game_art_unity as unity  # noqa: E402


def quaternion_product(a: list[float], b: list[float]) -> list[float]:
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]


def axis_turn(axis: int, degrees: float) -> list[float]:
    half = math.radians(degrees) / 2
    quaternion = [0.0, 0.0, 0.0, math.cos(half)]
    quaternion[axis] = math.sin(half)
    return quaternion


class PlainTests(unittest.TestCase):
    def test_references_are_handed_over_and_numbers_made_json(self):
        tree = {
            "m_Material": {"m_FileID": 0, "m_PathID": 7},
            "speed": 0.30000001192092896,
            "broken": float("nan"),
            "raw": b"\x01\x02",
            "list": [{"m_FileID": 1, "m_PathID": 0}],
        }

        converted = unity.plain(tree, lambda file_id, path_id: f"{file_id}:{path_id}")

        self.assertEqual(
            converted,
            {"m_Material": "0:7", "speed": 0.3, "broken": None, "raw": "AQI=", "list": ["1:0"]},
        )


class ParticleTests(unittest.TestCase):
    def test_switched_off_modules_are_left_out(self):
        system = {"InitialModule": {"startSpeed": 1}, "NoiseModule": {"enabled": False}, "SizeModule": {"enabled": True}}

        kept = unity.enabled_modules(system)

        self.assertEqual(sorted(kept), ["InitialModule", "SizeModule"])

    def test_a_constant_keeps_only_its_scalar(self):
        curve = {"minMaxState": 0, "scalar": 2.0, "minScalar": 1.0, "maxCurve": {}, "minCurve": {}}

        self.assertEqual(unity.prune_curves(curve), {"minMaxState": 0, "scalar": 2.0})

    def test_two_constants_keep_both_scalars(self):
        curve = {"minMaxState": 3, "scalar": 2.0, "minScalar": 1.0, "maxCurve": {}, "minCurve": {}}

        self.assertEqual(unity.prune_curves(curve), {"minMaxState": 3, "scalar": 2.0, "minScalar": 1.0})

    def test_a_gradient_keeps_only_its_gradient(self):
        colour = {"minMaxState": 1, "minColor": {}, "maxColor": {}, "minGradient": {}, "maxGradient": {"keys": 2}}

        self.assertEqual(unity.prune_curves(colour), {"minMaxState": 1, "maxGradient": {"keys": 2}})


class TransformSpaceTests(unittest.TestCase):
    def test_unity_values_mirror_into_gltf_space(self):
        translation_x = unity.gltf_component(1, 0, 2.0)
        rotation_y = unity.gltf_component(2, 1, 0.5)
        rotation_w = unity.gltf_component(2, 3, 0.5)
        scale_x = unity.gltf_component(3, 0, 2.0)

        self.assertEqual((translation_x, rotation_y, rotation_w, scale_x), (-2.0, -0.5, 0.5, 2.0))

    def test_euler_angles_turn_about_z_then_x_then_y(self):
        degrees = [30.0, 45.0, 60.0]
        expected = quaternion_product(
            quaternion_product(axis_turn(1, degrees[1]), axis_turn(0, degrees[0])),
            axis_turn(2, degrees[2]),
        )

        quaternion = uma.euler_quaternion(degrees)

        for got, wanted in zip(quaternion, expected):
            self.assertAlmostEqual(got, wanted)


if __name__ == "__main__":
    unittest.main()
