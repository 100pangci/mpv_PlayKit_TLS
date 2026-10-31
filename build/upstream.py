from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

from .common import BuildError, canonical_hash, fail, log, scoped_license_fields, sha256_bytes

API_ROOT = "https://api.github.com"
USER_AGENT = "mpv-PlayKit-build/1.0"


class GitHubNotFound(RuntimeError):
    pass


def github_token() -> str:
    return os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""


def github_headers(accept: str = "application/vnd.github+json") -> dict[str, str]:
    headers = {
        "Accept": accept,
        "User-Agent": USER_AGENT,
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = github_token()
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def github_request(url: str, *, accept: str = "application/vnd.github+json", timeout: int = 60) -> bytes:
    request = urllib.request.Request(url, headers=github_headers(accept))
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.read()
    except urllib.error.HTTPError as exc:
        body = exc.read(2000).decode("utf-8", errors="replace")
        if exc.code == 404:
            raise GitHubNotFound(f"GitHub 404: {url}\n{body}") from exc
        raise BuildError(f"GitHub HTTP {exc.code} for {url}\n{body}") from exc
    except urllib.error.URLError as exc:
        raise BuildError(f"GitHub request failed for {url}: {exc}") from exc


def github_json(url: str) -> Any:
    return json.loads(github_request(url).decode("utf-8"))


def get_release(repository: str, mode: str, tag: str | None = None) -> dict[str, Any]:
    if mode == "latest":
        url = f"{API_ROOT}/repos/{repository}/releases/latest"
    elif mode == "tag":
        if not tag:
            fail(f"{repository}: release.mode=tag requires release.tag")
        url = f"{API_ROOT}/repos/{repository}/releases/tags/{urllib.parse.quote(tag, safe='')}"
    else:
        fail(f"{repository}: unsupported release mode {mode!r}")
    release = github_json(url)
    if not isinstance(release, dict) or "tag_name" not in release:
        fail(f"{repository}: release response is invalid")
    return release


def list_releases(repository: str, per_page: int = 100) -> list[dict[str, Any]]:
    url = f"{API_ROOT}/repos/{repository}/releases?per_page={per_page}"
    releases = github_json(url)
    if not isinstance(releases, list):
        fail(f"{repository}: release list response is invalid")
    return releases


def download_asset_bytes(asset: dict[str, Any]) -> bytes:
    """Download a release asset through the API so private repos work."""
    url = asset.get("url")
    if not url:
        fail("asset has no API URL")
    return github_request(url, accept="application/octet-stream")


class StrictFormat(dict[str, Any]):
    def __missing__(self, key: str) -> str:
        raise BuildError(f"template references missing value {key!r}")


def render(template: str, context: dict[str, Any]) -> str:
    if not template:
        return ""
    try:
        return template.format_map(StrictFormat(context))
    except BuildError:
        raise
    except (KeyError, ValueError) as exc:
        raise BuildError(f"could not render template {template!r}: {exc}") from exc


def resolve_component(component: dict[str, Any]) -> dict[str, Any]:
    identifier = component["id"]
    repository = component["upstream"]
    release_config = component.get("release") or {}
    mode = release_config.get("mode", "latest")
    release = get_release(repository, mode, release_config.get("tag"))
    release_tag = str(release["tag_name"])

    pattern = re.compile(component["asset"]["pattern"])
    matches = [asset for asset in release.get("assets", []) if pattern.match(str(asset.get("name", "")))]
    if len(matches) != 1:
        names = ", ".join(str(asset.get("name")) for asset in release.get("assets", []))
        raise BuildError(
            f"{identifier}: expected exactly one asset matching {pattern.pattern!r} "
            f"in {repository}@{release_tag}, found {len(matches)}. Assets: {names}"
        )
    asset = matches[0]
    asset_name = str(asset["name"])
    match = pattern.match(asset_name)
    assert match is not None
    groups = {key: value for key, value in match.groupdict().items() if value is not None}

    context: dict[str, Any] = {
        "release_tag": release_tag,
        "asset_name": asset_name,
        "repository": repository,
        **groups,
    }
    version = render(component["asset"]["version"], context)
    commit = render(component["asset"].get("commit", ""), context) or None

    license_info = component["license"]
    license_files = []
    for item in license_info.get("files", []):
        license_files.append(
            {
                "path": item["path"],
                "license": item.get("license"),
                "url": render(item["url"], context),
            }
        )

    digest = asset.get("digest")
    sha256 = None
    if isinstance(digest, str) and digest.startswith("sha256:"):
        sha256 = digest.split(":", 1)[1].lower()
        if not re.fullmatch(r"[0-9a-f]{64}", sha256):
            fail(f"{identifier}: invalid sha256 digest {digest!r}")

    browser_url = asset.get("browser_download_url")
    locked = {
        "id": identifier,
        "kind": component.get("kind", "binary"),
        "platforms": component.get("platforms", "all"),
        "upstream": repository,
        "release_mode": mode,
        "release_tag": release_tag,
        "release_url": release.get("html_url"),
        "published_at": release.get("published_at"),
        "version": version,
        "commit": commit,
        "asset_name": asset_name,
        "asset_id": asset.get("id"),
        "api_url": asset.get("url"),
        "url": browser_url,
        "sha256": sha256,
        "size": asset.get("size"),
        "license": {
            "spdx": license_info.get("spdx"),
            "files": license_files,
            **scoped_license_fields(license_info),
        },
    }
    if not locked["sha256"]:
        # Older GitHub Enterprise instances may not expose asset digests.  Fall
        # back to downloading once so the lock stays complete and reproducible.
        api_url = locked.get("api_url")
        if not api_url:
            fail(f"{identifier}: asset has no digest and no API URL")
        log(f"{identifier}: digest missing, downloading asset to compute sha256")
        payload = github_request(api_url, accept="application/octet-stream", timeout=180)
        locked["sha256"] = sha256_bytes(payload)
        locked["size"] = len(payload)
    log(f"resolved {identifier}: {version} <- {repository}@{release_tag} / {asset_name}")
    return locked


def build_lock(
    repository: str,
    components: dict[str, dict[str, Any]],
    generated_at: str,
) -> dict[str, Any]:
    resolved = {identifier: resolve_component(component) for identifier, component in sorted(components.items())}
    return {
        "schema": 1,
        "generated_at": generated_at,
        "repository": repository,
        "components": resolved,
    }


def lock_content(lock: dict[str, Any]) -> dict[str, Any]:
    return {"components": lock.get("components", {})}


def lock_hash(lock: dict[str, Any]) -> str:
    return canonical_hash(lock_content(lock))


def component_summary(lock: dict[str, Any]) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    for identifier, component in sorted(lock.get("components", {}).items()):
        summary[identifier] = {
            "version": component.get("version"),
            "upstream": component.get("upstream"),
            "release_tag": component.get("release_tag"),
            "release_url": component.get("release_url"),
            "asset_name": component.get("asset_name"),
            "sha256": component.get("sha256"),
            "license": (component.get("license") or {}).get("spdx"),
        }
    return summary
