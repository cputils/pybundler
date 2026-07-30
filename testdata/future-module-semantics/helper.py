"""helper documentation"""

from __future__ import annotations


def annotated(value: MissingType) -> MissingType:
    return value


def annotation_name():
    return annotated.__annotations__["value"]
