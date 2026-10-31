from __future__ import annotations

import re
from datetime import datetime, timezone


def parse_release_version(value: str) -> tuple[int, ...] | None:
    text = str(value or "").strip()
    if text.startswith("v"):
        text = text[1:]
    if not re.fullmatch(r"\d+(?:\.\d+){0,3}", text):
        return None
    return tuple(int(part) for part in text.split("."))


def compare_versions(left: str, right: str) -> int:
    a = parse_release_version(left)
    b = parse_release_version(right)
    if a is None or b is None:
        raise ValueError(f"cannot compare non-numeric versions: {left!r}, {right!r}")
    for index in range(max(len(a), len(b))):
        x = a[index] if index < len(a) else 0
        y = b[index] if index < len(b) else 0
        if x != y:
            return 1 if x > y else -1
    return 0


def monthly_base(now: datetime | None = None) -> str:
    current = now or datetime.now(tz=timezone.utc)
    return f"{current.year}.{current.month:02d}"


def next_release_version(tags: list[str], now: datetime | None = None) -> str:
    base = monthly_base(now)
    base_parts = parse_release_version(base)
    assert base_parts is not None
    used: list[tuple[int, ...]] = []
    for tag in tags:
        parts = parse_release_version(tag)
        if parts is None:
            continue
        if parts[:2] == base_parts[:2] and len(parts) <= 3:
            used.append(parts)
    if not used:
        return base
    highest_patch = max((parts[2] if len(parts) >= 3 else 0) for parts in used)
    return f"{base}.{highest_patch + 1}"


def tag_for_version(version: str, prefix: str = "v") -> str:
    return f"{prefix}{version}"
