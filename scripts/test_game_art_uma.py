"""Run in the service image, which has UnityPy:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_uma
"""
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).parent))

import game_art_uma as uma  # noqa: E402


def translation(x: float, y: float, z: float) -> uma.Matrix:
    return [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z], [0.0, 0.0, 0.0, 1.0]]


QUARTER_TURN_ABOUT_Y: uma.Matrix = [
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
]


def mesh(vertices, weights, indices, normals=None):
    return SimpleNamespace(
        m_VertexCount=len(vertices),
        m_Vertices=vertices,
        m_Normals=normals or [(0.0, 0.0, 1.0)] * len(vertices),
        m_BoneWeights=weights,
        m_BoneIndices=indices,
    )


class VertexBoneTests(unittest.TestCase):
    def test_two_weights_per_vertex_are_read_as_pairs(self):
        handler = mesh([(0, 0, 0)] * 2, [(0.6, 0.4), (1.0, 0.0)], [(1, 2), (3, 0)])

        bones = uma.vertex_bones(handler)

        self.assertEqual(bones, [((1, 0.6), (2, 0.4)), ((3, 1.0),)])

    def test_an_unweighted_vertex_follows_the_first_bone(self):
        handler = mesh([(0, 0, 0)] * 2, [(0.0, 0.0, 0.0, 0.0)], [(5, 0, 0, 0)])

        bones = uma.vertex_bones(handler)

        self.assertEqual(bones, [((0, 1.0),), ((0, 1.0),)])


class RestPoseTests(unittest.TestCase):
    def test_a_mesh_modelled_at_one_offset_is_shifted_onto_the_rest(self):
        handler = mesh([(0.0, 0.0, 0.0)], [(1.0, 0.0)], [(0, 1)])
        offsets = [translation(0.1, 1.3, 0.0), translation(0.1, 1.3, 0.0)]

        uma.pose_to_rest(handler, offsets, uma.vertex_bones(handler))

        # Unity mirrors glTF's X.
        self.assertEqual(handler.m_Vertices, [(-0.1, 1.3, 0.0)])
        self.assertEqual(handler.m_Normals, [(0.0, 0.0, 1.0)])

    def test_a_bone_bound_rotated_poses_only_its_vertices(self):
        handler = mesh([(1.0, 0.0, 0.0), (1.0, 0.0, 0.0)], [(1.0,), (1.0,)], [(0,), (1,)])
        offsets = [QUARTER_TURN_ABOUT_Y, translation(0.0, 0.0, 0.0)]

        uma.pose_to_rest(handler, offsets, uma.vertex_bones(handler))

        turned, kept = handler.m_Vertices
        turned_normal, kept_normal = handler.m_Normals
        self.assertEqual(turned, (0.0, 0.0, 1.0))
        self.assertEqual(kept, (1.0, 0.0, 0.0))
        self.assertEqual(turned_normal, (-1.0, 0.0, 0.0))
        self.assertEqual(kept_normal, (0.0, 0.0, 1.0))


if __name__ == "__main__":
    unittest.main()
