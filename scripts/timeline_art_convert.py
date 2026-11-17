"""Fits official banner pictures to the size the timeline shows and writes them as WebP.

The resources service sends the jobs as JSON on stdin:
{"jobs": [{"source": "<path>", "output": "<path>", "fit": "japanese"}
          | {"source", "output", "fit": "english", "target": [width, height] | null,
             "reference": "<path>" | null, "reference_is_japanese": bool}]}
Japanese art is scaled down to at most 512 px wide. English art is fitted into the size of the
banner's Japanese art: `target`, else the size of `reference` (fitted as Japanese art when
`reference_is_japanese`), else 512x200. When it is wider than that size it is stretched up to
10% in height rather than cropped. Pictures with alpha are stored lossless.
It prints {"results": [{"output": "<path>", "size": [width, height], "error": null | "<message>"}]}.
"""

import json
import sys
from typing import Any

from PIL import Image

JAPANESE_MAX_WIDTH = 512
MAX_ENGLISH_STRETCH = 1.1
DEFAULT_TARGET = (512, 200)
WEBP_QUALITY = 88


def js_round(value: float) -> int:
    """Rounds halves up, as the frontend's sync did."""
    return int(value + 0.5)


def japanese_size(width: int, height: int) -> tuple[int, int]:
    if width <= JAPANESE_MAX_WIDTH:
        return width, height
    return JAPANESE_MAX_WIDTH, js_round(height * JAPANESE_MAX_WIDTH / width)


def english_size(width: int, height: int, target: tuple[int, int]) -> tuple[int, int]:
    target_width, target_height = target
    scale = min(target_width / width, target_height / height)
    fitted_width = js_round(width * scale)
    fitted_height = js_round(height * scale)

    source_aspect = width / height
    target_aspect = target_width / target_height
    if source_aspect > target_aspect:
        stretch = min(source_aspect / target_aspect, MAX_ENGLISH_STRETCH)
        fitted_height = min(target_height, js_round(fitted_height * stretch))
    return fitted_width, fitted_height


def has_alpha(image: Image.Image) -> bool:
    return image.mode in ("RGBA", "LA", "PA") or (image.mode == "P" and "transparency" in image.info)


def english_target(job: dict[str, Any]) -> tuple[int, int]:
    if job.get("target"):
        return tuple(job["target"])
    reference = job.get("reference")
    if not reference:
        return DEFAULT_TARGET
    with Image.open(reference) as image:
        width, height = image.size
    if job.get("reference_is_japanese"):
        return japanese_size(width, height)
    return width, height


def convert(job: dict[str, Any]) -> tuple[int, int]:
    with Image.open(job["source"]) as image:
        image.load()
        alpha = has_alpha(image)
        width, height = image.size
        if job["fit"] == "japanese":
            size, method = japanese_size(width, height), 3
        elif job["fit"] == "english":
            size, method = english_size(width, height, english_target(job)), 6
        else:
            raise ValueError(f"unknown fit {job['fit']!r}")

        picture = image.convert("RGBA" if alpha else "RGB")
        if picture.size != size:
            picture = picture.resize(size, Image.Resampling.LANCZOS)
        picture.save(job["output"], "WEBP", quality=WEBP_QUALITY, lossless=alpha, method=method)
    return size


def convert_all(jobs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    results = []
    for job in jobs:
        try:
            size = convert(job)
            results.append({"output": job["output"], "size": list(size), "error": None})
        except Exception as error:  # noqa: BLE001 - one broken picture must not stop the batch
            results.append({"output": job["output"], "size": None, "error": f"{type(error).__name__}: {error}"})
    return results


def main() -> None:
    request = json.load(sys.stdin)
    json.dump({"results": convert_all(request["jobs"])}, sys.stdout)


if __name__ == "__main__":
    main()
