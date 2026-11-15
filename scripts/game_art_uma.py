"""Exports umas and race motions as GLB files that fit each other.

A uma joins an outfit's body, head and tail prefabs on one skeleton in its rest pose, a T-pose.
Bones the prefabs share, such as the neck and the tail root, are merged by name. Heads and tails
are modelled at the origin, so their meshes are moved onto the T-pose and every skin binds to
the rest pose.

Every body and every tail share one rest skeleton, so a race motion is exported on Special
Week's racewear and the common tail, without meshes, and fits any uma.
"""

import collections
import struct
import zlib
from array import array
from typing import Any

import game_art_model as gm

Matrix = list[list[float]]

TRANSFORM_BINDING = 4
CURVES_PER_ATTRIBUTE = {1: 3, 2: 4, 3: 3, 4: 3}
CHANNEL_PATHS = {1: "translation", 2: "rotation", 3: "scale"}
SAMPLE_RATE = 30
BIND_TOLERANCE = 1e-3
IDENTITY: Matrix = [[1.0 if row == column else 0.0 for column in range(4)] for row in range(4)]


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
        self.vertex_shift: list[float] | None = None

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

    def visit(self, reader: Any, node: int, merging: bool) -> None:
        self.node_of[gm.object_key(reader)] = node
        transform = reader.read()
        self.attach_renderers(transform.m_GameObject.read(), node)
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
            self.visit(child_reader, child_node, merging)

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

    def finish_skins(self) -> None:
        world = self.rest_world()
        for name, renderer in self.skinned:
            if not renderer.m_Enabled or renderer.m_Mesh.m_PathID == 0:
                continue

            mesh_reader = renderer.m_Mesh.deref()
            joints = [self.node_of[gm.object_key(bone.deref())] for bone in renderer.m_Bones]
            bind_poses = [gltf_matrix(pose) for pose in mesh_reader.read().m_BindPose]
            materials = tuple(self.material(pointer) for pointer in renderer.m_Materials)
            self.vertex_shift = bind_shift(name, joints, bind_poses, world)
            mesh = self.mesh(mesh_reader, materials)
            self.vertex_shift = None
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
        if self.vertex_shift is not None and handler.m_Vertices:
            x, y, z = self.vertex_shift
            # The shift is in glTF space; Unity's X is mirrored.
            handler.m_Vertices = [(v[0] - x, v[1] + y, v[2] + z) for v in handler.m_Vertices]
        attributes = super().vertex_attributes(handler)
        weights_by_vertex = getattr(handler, "m_BoneWeights", None)
        if attributes is None or not weights_by_vertex:
            return attributes

        count = handler.m_VertexCount
        joints = array("H")
        weights = array("f")
        for indices, values in zip(handler.m_BoneIndices[:count], weights_by_vertex[:count]):
            total = sum(values[:4])
            if total <= 0:
                indices, values, total = (0, 0, 0, 0), (1.0, 0.0, 0.0, 0.0), 1.0
            for index, value in zip(indices[:4], values[:4]):
                joints.append(index if value > 0 else 0)
                weights.append(value / total)
        attributes["JOINTS_0"] = self.gltf.accessor(joints, "VEC4", count, gm.ARRAY_BUFFER)
        attributes["WEIGHTS_0"] = self.gltf.accessor(weights, "VEC4", count, gm.ARRAY_BUFFER)
        return attributes

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

    def add_clip(self, clip: Any, name: str) -> int:
        """Samples the clip's transform curves onto the skeleton; returns how many channels."""
        node_by_hash = {zlib.crc32(path.encode()): node for node, path in self.node_paths.items() if path}
        stop = clip.m_MuscleClip.m_StopTime
        frame_count = max(2, round(stop * SAMPLE_RATE) + 1)
        times = [index / SAMPLE_RATE for index in range(frame_count)]
        time_accessor = self.animation_accessor(array("f", times), "SCALAR", frame_count, min=[0.0], max=[times[-1]])

        channels, samplers = [], []
        for (path_hash, attribute), components in clip_curves(clip).items():
            node = node_by_hash.get(path_hash)
            if node is None or attribute not in CHANNEL_PATHS:
                continue
            values = array("f")
            width = CURVES_PER_ATTRIBUTE[attribute]
            for time in times:
                vector = [sample(components.get(component), time) for component in range(width)]
                values.extend(convert(attribute, vector, self.gltf.document["nodes"][node]))
            kind = "VEC4" if attribute == 2 else "VEC3"
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


def bind_shift(
    mesh_name: str, joints: list[int], bind_poses: list[list[float]], world: dict[int, Matrix]
) -> list[float] | None:
    """How far, in glTF space, a mesh was modelled away from the T-pose: one translation shared
    by every bone, or `None` for a mesh without bones."""
    shifts = []
    for joint, column_major in zip(joints, bind_poses):
        inverse_bind = [[column_major[c * 4 + r] for c in range(4)] for r in range(4)]
        offset = multiply(world[joint], inverse_bind)
        rotation_error = max(abs(offset[r][c] - IDENTITY[r][c]) for r in range(3) for c in range(3))
        if rotation_error > BIND_TOLERANCE:
            raise ValueError(f"{mesh_name}: a bind pose is rotated away from the rest pose")
        shifts.append([offset[r][3] for r in range(3)])
    if not shifts:
        return None

    mean = [sum(shift[axis] for shift in shifts) / len(shifts) for axis in range(3)]
    spread = max(abs(shift[axis] - mean[axis]) for shift in shifts for axis in range(3))
    if spread > BIND_TOLERANCE:
        raise ValueError(f"{mesh_name}: bones disagree on the bind offset by {spread * 100:.2f} cm")
    return mean


def rest_value(attribute: int, node: dict[str, Any]) -> list[float]:
    if attribute == 1:
        return node.get("translation", [0.0, 0.0, 0.0])
    if attribute == 2:
        return node.get("rotation", [0.0, 0.0, 0.0, 1.0])
    return node.get("scale", [1.0, 1.0, 1.0])


def convert(attribute: int, vector: list[float | None], node: dict[str, Any]) -> list[float]:
    """A sampled Unity value in glTF space; a missing component keeps the rest pose."""
    if any(value is None for value in vector):
        return rest_value(attribute, node)
    if attribute == 1:
        return [-vector[0], vector[1], vector[2]]
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
    """Keys by (path hash, attribute) and component, from the clip's streamed, dense and
    constant parts, which number their curves in that order."""
    data = clip.m_MuscleClip.m_Clip.data
    streamed, dense, constant = data.m_StreamedClip, data.m_DenseClip, data.m_ConstantClip
    stop = clip.m_MuscleClip.m_StopTime
    keys: dict[int, list[tuple[float, float]]] = collections.defaultdict(list)

    raw = struct.pack(f"<{len(streamed.data)}I", *streamed.data)
    frames = []
    offset = 0
    while offset < len(raw):
        time, count = struct.unpack_from("<fi", raw, offset)
        offset += 8
        frame_keys = []
        for _ in range(count):
            index, _in_slope, _out_slope, _unused, value = struct.unpack_from("<i4f", raw, offset)
            offset += 20
            frame_keys.append((index, value))
        frames.append((time, frame_keys))
    # Streamed clips open and close with sentinel frames.
    for time, frame_keys in frames[1:-1]:
        for index, value in frame_keys:
            keys[index].append((time, value))

    stream_count = streamed.curveCount
    for frame in range(dense.m_FrameCount):
        time = dense.m_BeginTime + frame / dense.m_SampleRate
        for curve in range(dense.m_CurveCount):
            keys[stream_count + curve].append((time, dense.m_SampleArray[frame * dense.m_CurveCount + curve]))

    constant_start = stream_count + dense.m_CurveCount
    for curve, value in enumerate(constant.data):
        keys[constant_start + curve] = [(0.0, value), (stop, value)]

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


def export_uma(environment: Any, model: dict[str, Any], output: dict[str, Any]) -> None:
    """Writes `output["path"]`: the uma in its T-pose, named for Blender after its title, with
    its traits as the armature's custom properties."""
    exporter = CharacterExporter(
        environment, output["key"], tail_slot_textures(environment, model), with_renderers=True
    )
    for prefab in model["prefabs"]:
        exporter.add_prefab(prefab)
    exporter.finish_skins()
    if not exporter.gltf.document["skins"]:
        raise ValueError("the uma has no skinned mesh")

    root = exporter.gltf.document["nodes"][exporter.root]
    root["name"] = model["title"]
    root["extras"] = model["traits"]
    exporter.gltf.write(output["path"])


def export_motion(environment: Any, clip_bundle: str, model: dict[str, Any], output: dict[str, Any]) -> None:
    """Writes `output["path"]`: the clip on the shared skeleton. Blender names the armature, the
    action and the slot to pick after the title, and drops animation extras, so the game's clip
    name is kept on the root."""
    exporter = CharacterExporter(environment, output["key"], {}, with_renderers=False)
    for prefab in model["skeleton"]:
        exporter.add_prefab(prefab)

    clips = gm.bundle_objects(environment, clip_bundle, "AnimationClip")
    if not clips:
        raise ValueError("the bundle holds no animation clip")
    clip = clips[0].read()
    if exporter.add_clip(clip, model["title"]) == 0:
        raise ValueError(f"{clip.m_Name} moves none of the skeleton's bones")
    exporter.skeleton_skin()

    root = exporter.gltf.document["nodes"][exporter.root]
    root["name"] = model["title"]
    root["extras"] = {"gameClip": clip.m_Name}
    exporter.gltf.write(output["path"])
