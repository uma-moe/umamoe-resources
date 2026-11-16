"""Run in the service image, beside the uma converter's tests:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_uma test_game_art_face
"""
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import game_art_face as face  # noqa: E402


def xyz(x, y, z):
    return {"x": x, "y": y, "z": z}


def row(path, position=(0.0, 0.0, 0.0), rotation=(0.0, 0.0, 0.0), scale=(0.0, 0.0, 0.0), override=0):
    return {
        "_path": path,
        "_isValidScaleTransform": 0,
        "_position": xyz(*position),
        "_rotation": xyz(*rotation),
        "_scale": xyz(*scale),
        "IsOverrideTarget": override,
    }


def facial_target(eye_entries):
    """A target whose eyes hold the given entries (right and left rows); brows and mouth hold only `Base`."""
    return {
        "_eyeTarget": [{"_faceGroupInfo": [{"_trsArray": [right]}, {"_trsArray": [left]}]} for right, left in eye_entries],
        "_eyebrowTarget": [{"_faceGroupInfo": [{"_trsArray": []}, {"_trsArray": []}]}],
        "_mouthTarget": [{"_faceGroupInfo": [{"_trsArray": []}]}],
    }


class EulerRotationTests(unittest.TestCase):
    def test_a_turn_about_y_mirrors_into_gltf(self):
        rotation = face.euler_rotation(0.0, 90.0, 0.0)

        half = math.sqrt(0.5)
        self.assertEqual([round(value, 6) for value in rotation], [0.0, round(-half, 6), 0.0, round(half, 6)])

    def test_unity_turns_about_z_then_x_then_y(self):
        # Z takes the X axis to Y, X takes it on to Z and Y brings it back; other orders end elsewhere.
        x, y, z, w = face.euler_rotation(90.0, 90.0, 90.0)
        moved_x_axis = [1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)]

        self.assertEqual([round(value, 6) + 0.0 for value in moved_x_axis], [1.0, 0.0, 0.0])


class ExpressionPoseTests(unittest.TestCase):
    def setUp(self):
        base = (row("Eye_R", position=(0.04, 0.07, 0.08)), row("Eye_L", position=(-0.04, 0.07, 0.08)))
        half_closed = (row("Eye_R", position=(0.0, -0.01, 0.0), rotation=(10.0, 0.0, 0.0)), row("Eye_L"))
        closed = (row("Eye_R", scale=(-1.0, -1.0, -1.0), override=1), row("Eye_L", scale=(-1.0, -1.0, -1.0)))
        self.target = facial_target([base, half_closed, closed])
        self.rest_scales = {"Eye_R": [1.0, 1.0, 1.0], "Eye_L": [1.0, 1.0, 1.0]}

    def test_expressions_are_named_by_side_after_the_game_enums(self):
        _, expressions = face.expression_poses(self.target, self.rest_scales)

        names = [name for name, _ in expressions]
        self.assertEqual(names, ["EyeR HalfA", "EyeL HalfA", "EyeR CloseA", "EyeL CloseA"])

    def test_an_expression_adds_its_offset_to_the_base_pose(self):
        base_pose, expressions = face.expression_poses(self.target, self.rest_scales)

        half_closed_right = expressions[0][1]["Eye_R"]
        self.assertEqual(base_pose["Eye_R"]["translation"], [-0.04, 0.07, 0.08])
        self.assertEqual([round(value, 6) for value in half_closed_right["translation"]], [-0.04, 0.06, 0.08])
        self.assertEqual(half_closed_right["rotation"], face.euler_rotation(10.0, 0.0, 0.0))

    def test_only_override_rows_offset_the_scale(self):
        _, expressions = face.expression_poses(self.target, self.rest_scales)

        closed_right = expressions[2][1]["Eye_R"]
        closed_left = expressions[3][1]["Eye_L"]
        self.assertEqual(closed_right["scale"], [0.0, 0.0, 0.0])
        self.assertEqual(closed_left["scale"], [1.0, 1.0, 1.0])

    def test_bones_the_skeleton_lacks_are_left_out(self):
        base_pose, expressions = face.expression_poses(self.target, {"Eye_R": [1.0, 1.0, 1.0]})

        self.assertEqual(list(base_pose), ["Eye_R"])
        self.assertEqual(expressions[1], ("EyeL HalfA", {}))


def shift(x, y, z):
    """A move that offsets every rest position by the same amount."""
    return [[0.0, 0.0, 0.0, x], [0.0, 0.0, 0.0, y], [0.0, 0.0, 0.0, z], [0.0, 0.0, 0.0, 0.0]]


class MorphDeltaTests(unittest.TestCase):
    def test_a_vertex_follows_a_moved_bone_by_its_weight(self):
        positions = [(0.0, 1.0, 0.0), (0.0, 2.0, 0.0)]
        bones = [((0, 0.5), (1, 0.5)), ((1, 1.0),)]

        indices, deltas = face.morph_deltas(positions, bones, {0: shift(0.0, 0.0, 0.02)})

        self.assertEqual(indices, [0])
        self.assertEqual(deltas, [[0.0, 0.0, 0.01]])

    def test_a_move_below_the_tolerance_leaves_the_vertex_out(self):
        indices, deltas = face.morph_deltas([(0.0, 0.0, 0.0)], [((0, 1.0),)], {0: shift(0.0, 1e-8, 0.0)})

        self.assertEqual((indices, deltas), ([], []))


if __name__ == "__main__":
    unittest.main()
