from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .common import DEFAULT_LOCK, MANIFEST_DIR, fail, load_manifests, log, now_iso, read_json, write_json
from .inputs import compute_inputs_hash
from .upstream import build_lock, lock_hash


def changed_components(old: dict, new: dict) -> list[str]:
    old_components = old.get("components", {})
    new_components = new.get("components", {})
    changed = []
    for identifier in sorted(set(old_components) | set(new_components)):
        if old_components.get(identifier) != new_components.get(identifier):
            changed.append(identifier)
    return changed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Resolve upstream releases and write sources.lock.json")
    parser.add_argument("--manifests", type=Path, default=MANIFEST_DIR)
    parser.add_argument("--output", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--repository", default=None, help="override the GitHub repository in manifests/project.yml")
    parser.add_argument("--generated-at", default=None)
    parser.add_argument("--check", action="store_true", help="compare with --output instead of writing it")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    project, components = load_manifests(args.manifests)
    repository = args.repository or project.get("repository")
    if not repository:
        fail("manifests/project.yml has no repository")

    lock = build_lock(repository, components, args.generated_at or now_iso())
    lock["inputs_hash"] = compute_inputs_hash()

    summary = {
        "repository": repository,
        "lock_hash": lock_hash(lock),
        "inputs_hash": lock["inputs_hash"],
        "components": {
            identifier: {
                "version": item.get("version"),
                "release_tag": item.get("release_tag"),
                "sha256": item.get("sha256"),
            }
            for identifier, item in sorted(lock["components"].items())
        },
    }

    if args.check:
        if not args.output.is_file():
            fail(f"no lock file to compare: {args.output}")
        old = read_json(args.output)
        changed = changed_components(old, lock)
        summary["changed"] = changed
        if args.json:
            print(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True))
        else:
            log(f"lock changed: {', '.join(changed) if changed else 'no'}")
        return 1 if changed else 0

    write_json(args.output, lock)
    log(f"wrote {args.output} ({len(lock['components'])} components)")
    if args.json:
        print(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
