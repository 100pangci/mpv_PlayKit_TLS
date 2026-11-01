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
    parser.add_argument(
        "--only",
        action="append",
        default=None,
        metavar="ID",
        help="resolve only this component and merge it into an existing lock; may be repeated",
    )
    parser.add_argument("--check", action="store_true", help="compare with --output instead of writing it")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    project, components = load_manifests(args.manifests)
    repository = args.repository or project.get("repository")
    if not repository:
        fail("manifests/project.yml has no repository")

    selected_components = components
    requested: list[str] = []
    if args.only:
        requested = sorted(set(args.only))
        unknown = [identifier for identifier in requested if identifier not in components]
        if unknown:
            fail(f"unknown component(s) for --only: {', '.join(unknown)}")
        if not args.output.is_file():
            fail(f"--only requires an existing lock: {args.output}")
        selected_components = {identifier: components[identifier] for identifier in requested}

    lock = build_lock(repository, selected_components, args.generated_at or now_iso())
    if requested:
        # Target a lock update at new or changed components without refreshing
        # unrelated floating "latest" releases in the same commit.
        old = read_json(args.output)
        merged = dict(old.get("components", {}))
        for identifier in requested:
            merged[identifier] = lock["components"][identifier]
        lock["components"] = merged
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
