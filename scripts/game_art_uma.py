"""Exports umas and race motions as GLB files that fit each other.

A uma joins an outfit's body, head and tail prefabs on one skeleton in its rest pose, a T-pose.
Bones the prefabs share, such as the neck and the tail root, are merged by name. Heads and tails
are modelled at the origin, so their meshes are moved onto the T-pose and every skin binds to
the rest pose. The uma looks as it does at rest in the game: meshes the game switches off, such
as tears, are left out, and the blush is fully transparent. The face's expressions, which the
game poses with bones, become blend shapes (`game_art_face.py`).

Every body and every tail share one rest skeleton, so a race motion is exported on Special
Week's racewear and the common tail, without meshes, and fits any uma. Chibi and cut-in clips go
on their family's skeleton the same way, and may move its root. A camera or position clip moves
one object, exported as a root with a child per other object it moves, named by the CRC-32 of
its path. A clip's curves that move no transform, such as a camera's field of view, are kept in
the root's `extras.unity.curves` (`game_art_unity.clip_bindings`).

A chibi or prop is exported like a uma, without expressions, on its own skeleton, with the
textures the game loads at runtime, such as a chibi's face sheets, in their materials' slots.
"""

import collections
import math
import zlib
from array import array
from collections.abc import Iterable
from typing import Any

import game_art_face as face
import game_art_model as gm
import game_art_unity as unity

Matrix = list[list[float]]
# Per expression, the matrix moving a rest position by what the expression adds at each bone.
ExpressionMoves = list[tuple[str, dict[int, Matrix]]]

TRANSFORM_BINDING = unity.TRANSFORM_BINDING
CURVES_PER_ATTRIBUTE = unity.CURVES_PER_ATTRIBUTE
EULER = 4
CHANNEL_PATHS = {1: "translation", 2: "rotation", 3: "scale", EULER: "rotation"}
SAMPLE_RATE = 30
BIND_TOLERANCE = 1e-3
IDENTITY: Matrix = [[1.0 if row == column else 0.0 for column in range(4)] for row in range(4)]
# The game shows this mesh only while an uma blushes, multiplying its texture over the skin.
BLUSH_NODE = "M_Cheek"


def gltf_matrix(matrix: Any) -> list[float]:
    """A Unity matrix with X mirrored, column-major as glTF stores it."""
    sign = (-1, 1, 1, 1)
    values = []
    for column in range(4):
        for row in range(4):
            values.append(getattr(matrix, f"e{row}{column}") * sign[row] * sign[column])
    return values


def trs_matrix(node: dict[str, Any]) -> Matrix:
    tx, ty, tz = node.get("translation", [0.0, 0.0, 0.0])
    x, y, z, w = node.get("rotation", [0.0, 0.0, 0.0, 1.0])
    scale = node.get("scale", [1.0, 1.0, 1.0])
    rotation = [
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ]
    translation = (tx, ty, tz)
    rows = [[rotation[r][c] * scale[c] for c in range(3)] + [translation[r]] for r in range(3)]
    return rows + [[0.0, 0.0, 0.0, 1.0]]


def multiply(a: Matrix, b: Matrix) -> Matrix:
    return [[sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)] for r in range(4)]


def invert(matrix: Matrix) -> Matrix:
    work = [row[:] + IDENTITY[r][:] for r, row in enumerate(matrix)]
    for column in range(4):
        pivot = max(range(column, 4), key=lambda r: abs(work[r][column]))
        work[column], work[pivot] = work[pivot], work[column]
        divisor = work[column][column]
        work[column] = [value / divisor for value in work[column]]
        for r in range(4):
            if r != column:
                factor = work[r][column]
                work[r] = [a - factor * b for a, b in zip(work[r], work[column])]
    return [row[4:] for row in work]


def is_invertible(matrix: Matrix) -> bool:
    (a, b, c), (d, e, f), (g, h, i) = (row[:3] for row in matrix[:3])
    return abs(a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)) > 1e-12


class CharacterExporter(gm.PrefabExporter):
    """Merges prefabs into one skeleton and writes skins bound to its rest pose."""

    def __init__(
        self,
        environment: Any,
        glb_key: str,
        slot_textures: dict[str, dict[str, Any]],
        with_renderers: bool,
    ) -> None:
        super().__init__(environment, glb_key, [])
        self.gltf.document["skins"] = []
        self.gltf.document["animations"] = []
        self.slot_textures = slot_textures
        self.with_renderers = with_renderers
        self.bones: dict[str, int] = {}
        self.node_of: dict[tuple[str, int], int] = {}
        self.node_paths: dict[int, str] = {}
        self.skinned: list[tuple[str, Any]] = []
        self.filled_materials: set[int] = set()
        self.root: int | None = None
        self.bind_offsets: list[Matrix] = []
        self.face_poses: tuple[dict[str, face.Pose], list[tuple[str, dict[str, face.Pose]]]] | None = None
        self.joint_moves: ExpressionMoves = []
        self.morph_targets: tuple[list[str], list[dict[str, int]]] | None = None

    def add_prefab(self, prefab: dict[str, str]) -> None:
        roots = gm.bundle_objects(self.environment, prefab["bundle"], "Transform")
        roots = [reader for reader in roots if reader.read().m_Father.m_PathID == 0]
        if not roots:
            raise ValueError(f"{prefab['object']} holds no prefab")

        named = [r for r in roots if r.read().m_GameObject.read().m_Name == prefab["object"]]
        root = (named or roots)[0]
        is_first = self.root is None
        if is_first:
            self.root = self.gltf.add("nodes", {"name": prefab["object"]})
            self.gltf.document["scenes"][0]["nodes"].append(self.root)
            self.node_paths[self.root] = ""
        self.visit(root, self.root, merging=not is_first)

    def add_expressions(self, target: dict[str, Any]) -> None:
        """Bakes the head's facial target into blend shapes on the meshes `finish_skins` writes."""
        nodes = self.gltf.document["nodes"]
        rest_scales = {name: nodes[node].get("scale", [1.0, 1.0, 1.0]) for name, node in self.bones.items()}
        self.face_poses = face.expression_poses(target, rest_scales)

    def visit(self, reader: Any, node: int, merging: bool, parent_active: bool = True) -> None:
        self.node_of[gm.object_key(reader)] = node
        transform = reader.read()
        game_object = transform.m_GameObject.read()
        # Unity hides a switched-off object's children with it.
        active = parent_active and game_object.m_IsActive
        # The game switches these on for an expression, such as tears, and loads their textures then.
        if active:
            self.attach_renderers(game_object, node)

        for child_pointer in transform.m_Children:
            child_reader = child_pointer.deref()
            child = child_reader.read()
            name = child.m_GameObject.read().m_Name
            child_node = self.bones.get(name) if merging else None
            if child_node is None:
                child_node = self.gltf.add("nodes", {"name": name, **gm.node_transform(child)})
                parent = self.gltf.document["nodes"][node]
                parent.setdefault("children", []).append(child_node)
                parent_path = self.node_paths[node]
                self.node_paths[child_node] = f"{parent_path}/{name}" if parent_path else name
                self.bones.setdefault(name, child_node)
            self.visit(child_reader, child_node, merging, active)

    def attach_renderers(self, game_object: Any, node: int) -> None:
        if not self.with_renderers:
            return

        mesh = self.renderer_mesh(game_object)
        if mesh is not None:
            self.gltf.document["nodes"][node]["mesh"] = mesh
        for pair in game_object.m_Component:
            reader = pair.component.deref()
            if reader.type.name == "SkinnedMeshRenderer":
                self.skinned.append((game_object.m_Name, reader.read()))

    def rest_world(self) -> dict[int, Matrix]:
        nodes = self.gltf.document["nodes"]
        world: dict[int, Matrix] = {}

        def walk(node: int, parent_matrix: Matrix) -> None:
            matrix = multiply(parent_matrix, trs_matrix(nodes[node]))
            world[node] = matrix
            for child in nodes[node].get("children", []):
                walk(child, matrix)

        walk(self.root, IDENTITY)
        return world

    def bind_matrices(self, joints: list[int], world: dict[int, Matrix]) -> int:
        """An accessor of inverse bind matrices binding the joints to the rest pose."""
        matrices = array("f")
        for joint in joints:
            inverse = invert(world[joint])
            matrices.extend(inverse[r][c] for c in range(4) for r in range(4))
        accessor = {
            "bufferView": self.gltf.buffer_view(matrices.tobytes()),
            "componentType": gm.FLOAT,
            "count": len(joints),
            "type": "MAT4",
        }
        return self.gltf.add("accessors", accessor)

    def expression_moves(self, world: dict[int, Matrix]) -> ExpressionMoves:
        """For each expression, how it moves the bones below the face bones it poses, relative
        to the base pose."""
        if self.face_poses is None:
            return []

        base_pose, expressions = self.face_poses
        nodes = self.gltf.document["nodes"]
        parent_of = {child: node for node in world for child in nodes[node].get("children", [])}
        base_locals = {self.bones[bone]: trs_matrix(pose) for bone, pose in base_pose.items()}
        base_world = {**world, **subtree_world(nodes, world, parent_of, base_locals, base_locals)}
        inverse_rest: dict[int, Matrix] = {}

        moves_by_expression: ExpressionMoves = []
        for name, pose in expressions:
            posed = [self.bones[bone] for bone in pose]
            local = {**base_locals, **{self.bones[bone]: trs_matrix(trs) for bone, trs in pose.items()}}
            moves = {}
            for node, matrix in subtree_world(nodes, base_world, parent_of, local, posed).items():
                change = [[matrix[r][c] - base_world[node][r][c] for c in range(4)] for r in range(4)]
                is_moved = max(abs(value) for row in change for value in row) > face.MORPH_TOLERANCE
                if not is_moved or not is_invertible(world[node]):
                    continue
                if node not in inverse_rest:
                    inverse_rest[node] = invert(world[node])
                moves[node] = multiply(change, inverse_rest[node])
            moves_by_expression.append((name, moves))
        return moves_by_expression

    def finish_skins(self) -> None:
        world = self.rest_world()
        moves_by_expression = self.expression_moves(world)
        for name, renderer in self.skinned:
            if not renderer.m_Enabled or renderer.m_Mesh.m_PathID == 0:
                continue

            mesh_reader = renderer.m_Mesh.deref()
            joints = [self.node_of[gm.object_key(bone.deref())] for bone in renderer.m_Bones]
            bind_poses = [gltf_matrix(pose) for pose in mesh_reader.read().m_BindPose]
            materials = tuple(self.material(pointer) for pointer in renderer.m_Materials)
            self.bind_offsets = bind_offsets(joints, bind_poses, world)
            self.joint_moves = joint_moves(joints, moves_by_expression)
            mesh = self.mesh(mesh_reader, materials)
            self.bind_offsets = []
            self.joint_moves = []
            if mesh is None:
                continue

            skin = {
                "joints": joints,
                "inverseBindMatrices": self.bind_matrices(joints, world),
                "skeleton": self.root,
            }
            node = self.gltf.add("nodes", {"name": name, "mesh": mesh, "skin": self.gltf.add("skins", skin)})
            self.gltf.document["scenes"][0]["nodes"].append(node)

    def skeleton_skin(self) -> None:
        """A skin over the animated bones and their parents, so viewers import them as one
        armature even without a mesh. Every other node is dropped."""
        world = self.rest_world()
        nodes = self.gltf.document["nodes"]
        parent_of = {child: node for node in world for child in nodes[node].get("children", [])}
        animated = {
            channel["target"]["node"]
            for animation in self.gltf.document["animations"]
            for channel in animation["channels"]
        }
        bones: set[int] = set()
        for node in animated:
            while node is not None and node != self.root and node not in bones:
                bones.add(node)
                node = parent_of.get(node)

        # Hidden helpers are scaled to zero; they cannot be bones.
        joints = [node for node in world if node in bones and is_invertible(world[node])]
        skin = {"joints": joints, "inverseBindMatrices": self.bind_matrices(joints, world), "skeleton": self.root}
        self.gltf.add("skins", skin)
        self.keep_nodes({self.root, *bones})

    def keep_nodes(self, keep: set[int]) -> None:
        document = self.gltf.document
        kept = sorted(keep)
        index_of = {old: new for new, old in enumerate(kept)}
        nodes = []
        for old in kept:
            node = dict(document["nodes"][old])
            children = [index_of[child] for child in node.pop("children", []) if child in index_of]
            if children:
                node["children"] = children
            nodes.append(node)
        document["nodes"] = nodes

        for skin in document["skins"]:
            skin["joints"] = [index_of[joint] for joint in skin["joints"]]
            skin["skeleton"] = index_of[skin["skeleton"]]
        for animation in document["animations"]:
            for channel in animation["channels"]:
                channel["target"]["node"] = index_of[channel["target"]["node"]]
        for scene in document["scenes"]:
            scene["nodes"] = [index_of[node] for node in scene["nodes"] if node in index_of]
        self.root = index_of[self.root]

    def vertex_attributes(self, handler: Any) -> dict[str, int] | None:
        bones = vertex_bones(handler)
        if self.bind_offsets and handler.m_Vertices:
            pose_to_rest(handler, self.bind_offsets, bones)
        attributes = super().vertex_attributes(handler)
        if attributes is None or not getattr(handler, "m_BoneWeights", None):
            return attributes

        count = handler.m_VertexCount
        joints = array("H")
        weights = array("f")
        # Some meshes store two weights per vertex; glTF wants four.
        for pairs in bones:
            for index, weight in pairs + ((0, 0.0),) * (4 - len(pairs)):
                joints.append(index)
                weights.append(weight)
        attributes["JOINTS_0"] = self.gltf.accessor(joints, "VEC4", count, gm.ARRAY_BUFFER)
        attributes["WEIGHTS_0"] = self.gltf.accessor(weights, "VEC4", count, gm.ARRAY_BUFFER)
        if self.joint_moves:
            self.morph_targets = self.expression_targets(handler, bones)
        return attributes

    def expression_targets(
        self, handler: Any, bones: list[tuple[tuple[int, float], ...]]
    ) -> tuple[list[str], list[dict[str, int]]] | None:
        """The blend shapes of the expressions that move this mesh, and their names."""
        count = handler.m_VertexCount
        positions = [(-vertex[0], vertex[1], vertex[2]) for vertex in handler.m_Vertices[:count]]
        names = []
        targets = []
        for name, moves in self.joint_moves:
            indices, deltas = face.morph_deltas(positions, bones, moves)
            if indices:
                names.append(name)
                targets.append({"POSITION": self.sparse_vectors(count, indices, deltas)})
        return (names, targets) if names else None

    def sparse_vectors(self, count: int, indices: list[int], vectors: list[list[float]]) -> int:
        """A VEC3 accessor that is zero except at `indices`, the compact form of a blend shape."""
        index_values = array("H" if count <= 65535 else "I", indices)
        values = array("f", [component for vector in vectors for component in vector])
        stored = values.tolist()
        # Bounds cover the zeros of the vertices the shape leaves in place.
        zeros = [0.0] if len(indices) < count else []
        lowest = [min(stored[axis::3] + zeros) for axis in range(3)]
        highest = [max(stored[axis::3] + zeros) for axis in range(3)]
        accessor = {
            "componentType": gm.FLOAT,
            "count": count,
            "type": "VEC3",
            "min": lowest,
            "max": highest,
            "sparse": {
                "count": len(indices),
                "indices": {
                    "bufferView": self.gltf.buffer_view(index_values.tobytes()),
                    "componentType": gm.COMPONENT_TYPES[index_values.typecode],
                },
                "values": {"bufferView": self.gltf.buffer_view(values.tobytes())},
            },
        }
        return self.gltf.add("accessors", accessor)

    def mesh(self, mesh_reader: Any, materials: tuple[int | None, ...]) -> int | None:
        """Also gives the mesh the blend shapes `vertex_attributes` baked for it."""
        self.morph_targets = None
        index = super().mesh(mesh_reader, materials)
        if index is None or self.morph_targets is None:
            return index

        names, targets = self.morph_targets
        gltf_mesh = self.gltf.document["meshes"][index]
        for primitive in gltf_mesh["primitives"]:
            primitive["targets"] = targets
        gltf_mesh["extras"] = {"targetNames": names}
        return index

    def material(self, pointer: Any) -> int | None:
        """Also puts the textures the game loads at runtime, a tail's colours, in their slots."""
        index = super().material(pointer)
        if index is None or index in self.filled_materials:
            return index

        self.filled_materials.add(index)
        entry = self.gltf.document["materials"][index]
        slots = entry["extras"]["unity"]["textures"]
        for slot, reader in self.slot_textures.get(entry["name"], {}).items():
            texture = reader.read()
            texture_index = self.gltf_texture(self.image(reader, texture), gm.texture_wrap(texture))
            slots[slot] = {"index": texture_index, **gm.IDENTITY_UV_TRANSFORM}
            if slot == "_MainTex":
                base_color = self.texture_info(texture_index, gm.IDENTITY_UV_TRANSFORM)
                entry["pbrMetallicRoughness"]["baseColorTexture"] = base_color
        return index

    def clip_targets(self, moves_root: bool) -> dict[int, int]:
        """Nodes by the CRC-32 of their path, as a clip names them; the root's path is empty."""
        targets = {zlib.crc32(path.encode()): node for node, path in self.node_paths.items() if path}
        if moves_root:
            targets[0] = self.root
        return targets

    def add_clip(self, clip: Any, name: str, targets: dict[int, int]) -> int:
        """Samples the clip's transform curves onto the nodes it names; returns how many
        channels."""
        stop = clip.m_MuscleClip.m_StopTime
        frame_count = max(2, round(stop * SAMPLE_RATE) + 1)
        times = [index / SAMPLE_RATE for index in range(frame_count)]
        time_accessor = self.animation_accessor(array("f", times), "SCALAR", frame_count, min=[0.0], max=[times[-1]])

        curves = clip_curves(clip)
        rotated = {path_hash for path_hash, attribute in curves if attribute == 2}
        channels, samplers = [], []
        for (path_hash, attribute), components in curves.items():
            node = targets.get(path_hash)
            if node is None or attribute not in CHANNEL_PATHS:
                continue
            # A quaternion curve of the same object wins over its Euler angles.
            if attribute == EULER and path_hash in rotated:
                continue
            values = array("f")
            width = CURVES_PER_ATTRIBUTE[attribute]
            for time in times:
                vector = [sample(components.get(component), time) for component in range(width)]
                values.extend(convert(attribute, vector, self.gltf.document["nodes"][node]))
            kind = "VEC3" if CHANNEL_PATHS[attribute] != "rotation" else "VEC4"
            samplers.append({
                "input": time_accessor,
                "output": self.animation_accessor(values, kind, frame_count),
                "interpolation": "LINEAR",
            })
            channels.append({"sampler": len(samplers) - 1, "target": {"node": node, "path": CHANNEL_PATHS[attribute]}})
        self.gltf.add("animations", {"name": name, "channels": channels, "samplers": samplers})
        return len(channels)

    def animation_accessor(self, values: array, kind: str, count: int, **fields: Any) -> int:
        # Animation data is not vertex data, so its buffer view has no target.
        accessor = {
            "bufferView": self.gltf.buffer_view(values.tobytes()),
            "componentType": gm.FLOAT,
            "count": count,
            "type": kind,
            **fields,
        }
        return self.gltf.add("accessors", accessor)


def bind_offsets(joints: list[int], bind_poses: list[list[float]], world: dict[int, Matrix]) -> list[Matrix]:
    """Per bone, the glTF-space matrix moving a vertex from where the mesh was modelled to where
    the bone puts it in the T-pose."""
    offsets = []
    for joint, column_major in zip(joints, bind_poses):
        inverse_bind = [[column_major[c * 4 + r] for c in range(4)] for r in range(4)]
        offsets.append(multiply(world[joint], inverse_bind))
    return offsets


def subtree_world(
    nodes: list[dict[str, Any]],
    world: dict[int, Matrix],
    parent_of: dict[int, int],
    local: dict[int, Matrix],
    posed: Iterable[int],
) -> dict[int, Matrix]:
    """The world matrices below the `posed` nodes, where `local` replaces some nodes' own
    transforms; `world` holds those of everything else."""
    posed_nodes = set(posed)
    changed: dict[int, Matrix] = {}

    def has_posed_ancestor(node: int) -> bool:
        parent = parent_of.get(node)
        while parent is not None:
            if parent in posed_nodes:
                return True
            parent = parent_of.get(parent)
        return False

    def walk(node: int, parent_matrix: Matrix) -> None:
        own = local[node] if node in local else trs_matrix(nodes[node])
        matrix = multiply(parent_matrix, own)
        changed[node] = matrix
        for child in nodes[node].get("children", []):
            walk(child, matrix)

    for node in posed_nodes:
        if not has_posed_ancestor(node):
            walk(node, world[parent_of[node]])
    return changed


def joint_moves(joints: list[int], moves_by_expression: ExpressionMoves) -> ExpressionMoves:
    """The expressions that move any of a mesh's joints, with their moves by the mesh's bone index."""
    by_bone_index = []
    for name, moves in moves_by_expression:
        bone_moves = {index: moves[node] for index, node in enumerate(joints) if node in moves}
        if bone_moves:
            by_bone_index.append((name, bone_moves))
    return by_bone_index


def unity_matrix(matrix: Matrix) -> Matrix:
    """A glTF-space matrix in Unity space, whose X is mirrored."""
    sign = (-1, 1, 1, 1)
    return [[matrix[r][c] * sign[r] * sign[c] for c in range(4)] for r in range(4)]


def vertex_bones(handler: Any) -> list[tuple[tuple[int, float], ...]]:
    """Each vertex's bones with positive weight, normalised; an unweighted vertex follows the
    first bone."""
    weights_by_vertex = getattr(handler, "m_BoneWeights", None) or []
    indices_by_vertex = getattr(handler, "m_BoneIndices", None) or []
    bones = []
    for vertex in range(handler.m_VertexCount):
        pairs: tuple[tuple[int, float], ...] = ()
        if vertex < len(weights_by_vertex):
            values = weights_by_vertex[vertex][:4]
            total = sum(value for value in values if value > 0)
            if total > 0:
                indices = indices_by_vertex[vertex][:4]
                pairs = tuple((index, value / total) for index, value in zip(indices, values) if value > 0)
        bones.append(pairs or ((0, 1.0),))
    return bones


def common_shift(offsets: list[Matrix]) -> list[float] | None:
    """The translation every offset shares when none of them rotates, else `None`."""
    for offset in offsets:
        rotation_error = max(abs(offset[r][c] - IDENTITY[r][c]) for r in range(3) for c in range(3))
        if rotation_error > BIND_TOLERANCE:
            return None

    shifts = [[offset[r][3] for r in range(3)] for offset in offsets]
    mean = [sum(shift[axis] for shift in shifts) / len(shifts) for axis in range(3)]
    spread = max(abs(shift[axis] - mean[axis]) for shift in shifts for axis in range(3))
    return mean if spread <= BIND_TOLERANCE else None


def pose_to_rest(handler: Any, offsets: list[Matrix], bones: list[tuple[tuple[int, float], ...]]) -> None:
    """Moves the vertices and normals to the T-pose the way the game skins them there, so the
    skin can bind to the rest pose. Most meshes were modelled at one offset from it, such as a
    head at the origin; a few have bones bound rotated or apart."""
    count = handler.m_VertexCount
    if count == 0:
        return

    unity_offsets = [unity_matrix(offset) for offset in offsets]
    weighted = sorted({index for pairs in bones for index, _ in pairs})
    if weighted[-1] >= len(unity_offsets):
        raise ValueError("a vertex is weighted to a bone the mesh does not bind")

    shift = common_shift([unity_offsets[index] for index in weighted])
    if shift is not None:
        x, y, z = shift
        handler.m_Vertices = [(v[0] + x, v[1] + y, v[2] + z) for v in handler.m_Vertices[:count]]
        return

    vertices, normals = [], []
    source_normals = handler.m_Normals or []
    for vertex, pairs in enumerate(bones):
        blend = [[sum(weight * unity_offsets[index][r][c] for index, weight in pairs) for c in range(4)] for r in range(3)]
        x, y, z = handler.m_Vertices[vertex][:3]
        vertices.append(tuple(row[0] * x + row[1] * y + row[2] * z + row[3] for row in blend))
        if vertex < len(source_normals):
            nx, ny, nz = source_normals[vertex][:3]
            normals.append(tuple(row[0] * nx + row[1] * ny + row[2] * nz for row in blend))
    handler.m_Vertices = vertices
    if len(normals) == count:
        handler.m_Normals = normals


def rest_value(attribute: int, node: dict[str, Any]) -> list[float]:
    if attribute == 1:
        return node.get("translation", [0.0, 0.0, 0.0])
    if attribute in (2, EULER):
        return node.get("rotation", [0.0, 0.0, 0.0, 1.0])
    return node.get("scale", [1.0, 1.0, 1.0])


def euler_quaternion(degrees: list[float]) -> list[float]:
    """Unity Euler angles in degrees as its quaternion; Unity turns about Z, then X, then Y."""
    half = [math.radians(angle) / 2 for angle in degrees]
    sx, sy, sz = (math.sin(angle) for angle in half)
    cx, cy, cz = (math.cos(angle) for angle in half)
    return [
        cy * sx * cz + sy * cx * sz,
        sy * cx * cz - cy * sx * sz,
        cy * cx * sz - sy * sx * cz,
        cy * cx * cz + sy * sx * sz,
    ]


def convert(attribute: int, vector: list[float | None], node: dict[str, Any]) -> list[float]:
    """A sampled Unity value in glTF space; a missing component keeps the rest pose."""
    if any(value is None for value in vector):
        return rest_value(attribute, node)
    if attribute == 1:
        return [-vector[0], vector[1], vector[2]]
    if attribute == EULER:
        return convert(2, euler_quaternion(vector), node)
    if attribute == 2:
        x, y, z, w = vector[0], -vector[1], -vector[2], vector[3]
        length = (x * x + y * y + z * z + w * w) ** 0.5 or 1.0
        return [x / length, y / length, z / length, w / length]
    return vector


def sample(keys: list[tuple[float, float]] | None, time: float) -> float | None:
    if not keys:
        return None
    if time <= keys[0][0]:
        return keys[0][1]
    for (start, first), (end, second) in zip(keys, keys[1:]):
        if start <= time <= end:
            if end == start:
                return second
            return first + (second - first) * (time - start) / (end - start)
    return keys[-1][1]


def clip_curves(clip: Any) -> dict[tuple[int, int], dict[int, list[tuple[float, float]]]]:
    """Transform keys by (path hash, attribute) and component."""
    keys = unity.curve_keys(clip)
    by_binding: dict[tuple[int, int], dict[int, list[tuple[float, float]]]] = collections.defaultdict(dict)
    index = 0
    for binding in clip.m_ClipBindingConstant.genericBindings:
        is_transform = binding.typeID == TRANSFORM_BINDING
        width = CURVES_PER_ATTRIBUTE.get(binding.attribute, 1) if is_transform else 1
        for component in range(width):
            if is_transform and index in keys:
                by_binding[(binding.path, binding.attribute)][component] = sorted(keys[index])
            index += 1
    return by_binding


def tail_slot_textures(environment: Any, model: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """The tail textures by the tail's material, `mtl_tail0001_00` for `pfb_tail0001_00`."""
    tails = [prefab["object"] for prefab in model["prefabs"] if prefab["object"].startswith("pfb_tail")]
    if not tails or not model["tail_textures"]:
        return {}

    material = "mtl_" + tails[0].removeprefix("pfb_")
    textures = {
        texture["slot"]: gm.bundle_texture(environment, texture["bundle"])
        for texture in model["tail_textures"]
    }
    return {material: textures}


def facial_target(environment: Any, model: dict[str, Any]) -> dict[str, Any] | None:
    """The head's FaceDrivenKeyTarget, which holds its expressions."""
    for prefab in model["prefabs"]:
        if not prefab["object"].startswith("pfb_chr"):
            continue
        for reader in gm.bundle_objects(environment, prefab["bundle"], "MonoBehaviour"):
            if reader.peek_name().endswith("_facial_target"):
                return reader.read_typetree()
    return None


def hide_blush(document: dict[str, Any]) -> None:
    """glTF cannot multiply, so viewers get the blush fully transparent, as the game shows it
    at rest. `extras.unity.blend` keeps how the game draws it."""
    for node in document["nodes"]:
        if node["name"] != BLUSH_NODE or "mesh" not in node:
            continue

        node["extras"] = {"active": False}
        for primitive in document["meshes"][node["mesh"]]["primitives"]:
            if "material" not in primitive:
                continue
            material = document["materials"][primitive["material"]]
            material["alphaMode"] = "BLEND"
            pbr = material["pbrMetallicRoughness"]
            red, green, blue, _ = pbr.get("baseColorFactor", [1.0, 1.0, 1.0, 1.0])
            pbr["baseColorFactor"] = [red, green, blue, 0.0]
            material["extras"]["unity"]["blend"] = "multiply"


def export_uma(environment: Any, model: dict[str, Any], output: dict[str, Any]) -> None:
    """Writes `output["path"]`: the uma in its T-pose, named for Blender after its title, with
    its traits as the armature's custom properties."""
    exporter = CharacterExporter(
        environment, output["key"], tail_slot_textures(environment, model), with_renderers=True
    )
    for prefab in model["prefabs"]:
        exporter.add_prefab(prefab)
    target = facial_target(environment, model)
    if target is not None:
        exporter.add_expressions(target)
    exporter.finish_skins()
    if not exporter.gltf.document["skins"]:
        raise ValueError("the uma has no skinned mesh")
    hide_blush(exporter.gltf.document)

    root = exporter.gltf.document["nodes"][exporter.root]
    root["name"] = model["title"]
    root["extras"] = model["traits"]
    exporter.gltf.write(output["path"])


def export_character(environment: Any, model: dict[str, Any], output: dict[str, Any]) -> None:
    """Writes `output["path"]`: a chibi or prop at rest, named after its title, with its traits
    on the root."""
    slot_textures: dict[str, dict[str, Any]] = {}
    for texture in model["material_textures"]:
        reader = gm.bundle_texture(environment, texture["bundle"])
        slot_textures.setdefault(texture["material"], {})[texture["slot"]] = reader
    exporter = CharacterExporter(environment, output["key"], slot_textures, with_renderers=True)
    for prefab in model["prefabs"]:
        exporter.add_prefab(prefab)
    exporter.finish_skins()
    document = exporter.gltf.document
    if not document["meshes"]:
        raise ValueError("the model draws no mesh")
    hide_blush(document)

    root = document["nodes"][exporter.root]
    root["name"] = model["title"]
    if model.get("traits"):
        root["extras"] = model["traits"]
    exporter.gltf.write(output["path"])


def bundle_clip(environment: Any, clip_bundle: str) -> Any:
    clips = gm.bundle_objects(environment, clip_bundle, "AnimationClip")
    if not clips:
        raise ValueError("the bundle holds no animation clip")
    return clips[0]


def untransformed_curves(clip_reader: Any) -> list[dict[str, Any]]:
    """The clip's bindings that move no transform, such as a field of view or a switch."""
    bindings = unity.clip_bindings(clip_reader.read(), unity.names_in(clip_reader.assets_file))
    return [binding for binding in bindings if binding["typeID"] != TRANSFORM_BINDING]


def clip_extras(clip_reader: Any) -> dict[str, Any]:
    extras: dict[str, Any] = {"gameClip": clip_reader.read().m_Name}
    curves = untransformed_curves(clip_reader)
    if curves:
        extras["unity"] = {"curves": curves}
    return extras


def export_motion(environment: Any, clip_bundle: str, model: dict[str, Any], output: dict[str, Any]) -> None:
    """Writes `output["path"]`: the clip on the shared skeleton. Blender names the armature, the
    action and the slot to pick after the title, and drops animation extras, so the game's clip
    name is kept on the root."""
    exporter = CharacterExporter(environment, output["key"], {}, with_renderers=False)
    for prefab in model["skeleton"]:
        exporter.add_prefab(prefab)

    clip_reader = bundle_clip(environment, clip_bundle)
    clip = clip_reader.read()
    targets = exporter.clip_targets(moves_root=model.get("moves_root", False))
    if exporter.add_clip(clip, model["title"], targets) == 0:
        raise ValueError(f"{clip.m_Name} moves none of the skeleton's bones")
    exporter.skeleton_skin()

    root = exporter.gltf.document["nodes"][exporter.root]
    root["name"] = model["title"]
    root["extras"] = clip_extras(clip_reader)
    exporter.gltf.write(output["path"])


def export_node_motion(environment: Any, clip_bundle: str, model: dict[str, Any], output: dict[str, Any]) -> None:
    """Writes `output["path"]`: a camera or position clip on a root and a child per other object
    it moves, named by the CRC-32 of that object's path."""
    exporter = CharacterExporter(environment, output["key"], {}, with_renderers=False)
    document = exporter.gltf.document
    exporter.root = exporter.gltf.add("nodes", {"name": model["title"]})
    document["scenes"][0]["nodes"].append(exporter.root)

    clip_reader = bundle_clip(environment, clip_bundle)
    clip = clip_reader.read()
    targets = {0: exporter.root}
    for path_hash, _attribute in clip_curves(clip):
        if path_hash not in targets:
            targets[path_hash] = exporter.gltf.add("nodes", {"name": f"{path_hash:08x}"})
            document["nodes"][exporter.root].setdefault("children", []).append(targets[path_hash])
    if exporter.add_clip(clip, model["title"], targets) == 0:
        raise ValueError(f"{clip.m_Name} moves no object")

    document["nodes"][exporter.root]["extras"] = clip_extras(clip_reader)
    exporter.gltf.write(output["path"])
