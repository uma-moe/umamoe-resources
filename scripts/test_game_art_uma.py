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


def transform(name, active=True, children=()):
    """A Unity transform reader with its game object and child transforms."""
    game_object = SimpleNamespace(m_Name=name, m_IsActive=active)
    unity_transform = SimpleNamespace(
        m_GameObject=SimpleNamespace(read=lambda: game_object),
        m_Children=[SimpleNamespace(deref=lambda child=child: child) for child in children],
        m_LocalPosition=SimpleNamespace(x=0.0, y=0.0, z=0.0),
        m_LocalRotation=SimpleNamespace(x=0.0, y=0.0, z=0.0, w=1.0),
        m_LocalScale=SimpleNamespace(x=1.0, y=1.0, z=1.0),
    )
    return SimpleNamespace(
        assets_file=SimpleNamespace(name="head"),
        path_id=id(unity_transform),
        read=lambda: unity_transform,
    )


class DrawnObjectsExporter(uma.CharacterExporter):
    def __init__(self):
        super().__init__(None, {}, with_renderers=True)
        self.drawn = []
        self.root = self.gltf.add("nodes", {"name": "root"})
        self.node_paths[self.root] = ""

    def attach_renderers(self, game_object, node):
        self.drawn.append(game_object.m_Name)


class SwitchedOffObjectTests(unittest.TestCase):
    def test_a_switched_off_object_and_its_children_draw_nothing(self):
        exporter = DrawnObjectsExporter()
        head = transform("Head", children=[
            transform("M_Tear", active=False, children=[transform("M_Tear_L")]),
            transform("M_Face"),
        ])

        exporter.visit(head, exporter.root, merging=False)

        self.assertEqual(exporter.drawn, ["Head", "M_Face"])


def material(name):
    return {"name": name, "pbrMetallicRoughness": {}, "extras": {"unity": {}}}


class BlushTests(unittest.TestCase):
    def test_the_blush_is_transparent_and_hidden_at_rest(self):
        document = {
            "nodes": [{"name": "M_Cheek", "mesh": 0}, {"name": "M_Face", "mesh": 1}],
            "meshes": [{"primitives": [{"material": 0}]}, {"primitives": [{"material": 1}]}],
            "materials": [material("mtl_chr1001_00_cheek"), material("mtl_chr1001_00_face")],
        }

        uma.hide_blush(document)

        blush, face = document["materials"]
        self.assertEqual(document["nodes"][0]["extras"], {"active": False})
        self.assertEqual(blush["alphaMode"], "BLEND")
        self.assertEqual(blush["pbrMetallicRoughness"]["baseColorFactor"], [1.0, 1.0, 1.0, 0.0])
        self.assertEqual(blush["extras"]["unity"]["blend"], "multiply")
        self.assertEqual(face, material("mtl_chr1001_00_face"))


if __name__ == "__main__":
    unittest.main()
