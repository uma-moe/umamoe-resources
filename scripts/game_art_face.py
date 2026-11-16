"""Facial expressions as glTF morph targets (blend shapes).

The game moves a face with bones, not blend shapes. Each head's FaceDrivenKeyTarget
(`ast_chr1001_00_facial_target`) lists the eye, eyebrow and mouth expressions; each holds, per
bone, an offset from the `Base` pose, and the game adds the offsets of the expressions it shows
by weight. Every expression is baked into the vertices its bones move, so a viewer gets one
blend shape per expression, named after the game client's enums: `EyeR CloseA`, `Mouth WaraiA`.
"""

import math
from typing import Any

Matrix = list[list[float]]
Pose = dict[str, list[float]]

# The game client's FaceEyeType, FaceEyebrowType and FaceMouthType, in the facial target's order.
FACE_EYE_TYPES = (
    "Base", "HalfA", "CloseA", "HalfB", "HalfC", "WaraiA", "WaraiB", "WaraiC", "WaraiD", "IkariA",
    "KanasiA", "DereA", "OdorokiA", "OdorokiB", "OdorokiC", "JitoA", "KusyoA", "UreiA", "RunA",
    "DrivenA", "XRange", "YRange", "EyeHideA", "SeriousA", "PupilA", "PupilB", "PupilC",
    "EyelidHideA", "EyelidHideB",
)
FACE_EYEBROW_TYPES = (
    "Base", "WaraiA", "WaraiB", "WaraiC", "WaraiD", "IkariA", "KanasiA", "DoyaA", "DereA",
    "OdorokiA", "OdorokiB", "JitoA", "KomariA", "KusyoA", "UreiA", "RunA", "RunB", "SeriousA",
    "SeriousB", "ShiwaA", "ShiwaB", "Offset_U", "Offset_D", "Offset_L", "Offset_R",
)
FACE_MOUTH_TYPES = (
    "Base", "Normal", "CheekA_L", "CheekA_R", "WaraiA", "WaraiB", "WaraiC", "WaraiD", "WaraiE",
    "IkariA", "IkariB", "KanasiA", "DoyaA", "DereA", "OdorokiA", "OdorokiB", "JitoA", "KomariA",
    "KusyoA_L", "KusyoA_R", "KusyoB_L", "KusyoB_R", "UreiA", "TalkA_A_S", "TalkA_A_L",
    "TalkA_I_S", "TalkA_I_L", "TalkA_U_S", "TalkA_U_L", "TalkA_E_S", "TalkA_E_L", "TalkA_O_S",
    "TalkA_O_L", "TalkB_A_S", "TalkB_A_L", "TalkB_I_S", "TalkB_I_L", "TalkB_E_S", "TalkB_E_L",
    "RunA", "RunB", "DrivenA", "ToothHide", "TalkC_I", "TanA", "TanB", "TanC_L", "TanD_L",
    "TanC_R", "TanD_R", "Offset_U", "Offset_D", "Offset_L", "Offset_R", "Scale_U", "Scale_D",
    "LowAngle",
)
# Each expression holds one bone group per side, named like the game's TargetInfomation.Part.
FACE_PARTS = (
    ("_eyeTarget", ("EyeR", "EyeL"), FACE_EYE_TYPES),
    ("_eyebrowTarget", ("EyebrowR", "EyebrowL"), FACE_EYEBROW_TYPES),
    ("_mouthTarget", ("Mouth",), FACE_MOUTH_TYPES),
)
MORPH_TOLERANCE = 1e-6


def quaternion_product(a: list[float], b: list[float]) -> list[float]:
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]


def euler_rotation(x: float, y: float, z: float) -> list[float]:
    """Unity's Quaternion.Euler, in degrees about Z, then X, then Y, as a glTF rotation."""

    def about(axis: int, degrees: float) -> list[float]:
        half = math.radians(degrees) / 2
        quaternion = [0.0, 0.0, 0.0, math.cos(half)]
        quaternion[axis] = math.sin(half)
        return quaternion

    qx, qy, qz, qw = quaternion_product(quaternion_product(about(1, y), about(0, x)), about(2, z))
    # glTF mirrors Unity's X.
    return [qx, -qy, -qz, qw]


def vector(value: dict[str, float]) -> list[float]:
    return [value["x"], value["y"], value["z"]]


def bone_pose(base: dict[str, Any], offset: dict[str, Any] | None, rest_scale: list[float]) -> Pose:
    """A bone's glTF local transform in the base pose, plus an expression's offset."""
    position = vector(base["_position"])
    rotation = vector(base["_rotation"])
    scale = list(rest_scale)
    if offset is not None:
        position = [a + b for a, b in zip(position, vector(offset["_position"]))]
        rotation = [a + b for a, b in zip(rotation, vector(offset["_rotation"]))]
        # Only override rows carry scale offsets; they shrink eyelid lines away.
        if offset["IsOverrideTarget"]:
            scale = [a + b for a, b in zip(scale, vector(offset["_scale"]))]

    x, y, z = position
    return {"translation": [-x, y, z], "rotation": euler_rotation(*rotation), "scale": scale}


def expression_poses(
    target: dict[str, Any], rest_scales: dict[str, list[float]]
) -> tuple[dict[str, Pose], list[tuple[str, dict[str, Pose]]]]:
    """The base pose of every face bone, and each expression's pose of the bones it moves.
    Bones the skeleton lacks, and expressions newer than the names above, are left out."""
    base_pose: dict[str, Pose] = {}
    expressions: list[tuple[str, dict[str, Pose]]] = []
    for field, sides, types in FACE_PARTS:
        entries = target[field]
        base_groups = [
            {row["_path"]: row for row in group["_trsArray"] if row["_path"] in rest_scales}
            for group in entries[0]["_faceGroupInfo"]
        ]
        for base_rows in base_groups:
            for path, row in base_rows.items():
                base_pose[path] = bone_pose(row, None, rest_scales[path])

        for type_name, entry in zip(types[1:], entries[1:]):
            for side, base_rows, group in zip(sides, base_groups, entry["_faceGroupInfo"]):
                pose = {
                    row["_path"]: bone_pose(base_rows[row["_path"]], row, rest_scales[row["_path"]])
                    for row in group["_trsArray"]
                    if row["_path"] in base_rows
                }
                expressions.append((f"{side} {type_name}", pose))
    return base_pose, expressions


def morph_deltas(
    positions: list[tuple[float, float, float]],
    bones: list[tuple[tuple[int, float], ...]],
    moves: dict[int, Matrix],
) -> tuple[list[int], list[list[float]]]:
    """The vertices an expression moves, and how far: each vertex follows its bones' moves by
    weight. A move maps a rest position to its offset."""
    indices: list[int] = []
    deltas: list[list[float]] = []
    for vertex, pairs in enumerate(bones):
        moving = [(moves[bone], weight) for bone, weight in pairs if bone in moves]
        if not moving:
            continue

        x, y, z = positions[vertex]
        delta = [
            sum(weight * (move[row][0] * x + move[row][1] * y + move[row][2] * z + move[row][3]) for move, weight in moving)
            for row in range(3)
        ]
        if max(abs(component) for component in delta) > MORPH_TOLERANCE:
            indices.append(vertex)
            deltas.append(delta)
    return indices, deltas
