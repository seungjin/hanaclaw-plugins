#!/usr/bin/env python3
"""Shared plugin packaging contract for the hanaclaw registry tools.

One implementation of the parts that must agree byte for byte with upstream
``zeroclaw-plugins`` (``tools/build-registry.py`` + ``tools/registry_contract.py``):

* the host's manifest vocabulary (capabilities, permissions, sender_match),
* the ``<name>@<version>`` identity rules,
* the reproducible package zip and its sha256.

``update_zeroclaw_registry.py`` (needs a prebuilt .wasm) and
``build_plugins.py`` (builds one) both import this module, so a digest
computed by either equals what the publish step uploads.

Standard library only. Run it through ``uv run --python 3.13 python``: a
zlib-ng interpreter compresses differently and changes every digest.
"""
from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import re
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.request
import zipfile
import zlib
from pathlib import Path

# ── host contract (zeroclaw crates/zeroclaw-plugins, registry_contract.py) ──

PLUGIN_RELEASE_TAG = "plugins"
# Mirrors src/plugin_registry.rs MAX_PLUGIN_ZIP_BYTES.
MAX_PLUGIN_ZIP_BYTES = 50 * 1024 * 1024
MAX_PLUGIN_EXTRACTED_BYTES = 50 * 1024 * 1024
# build-registry.py ZIP_COMPRESSION_LEVEL, pinned so digests never drift.
ZIP_COMPRESSION_LEVEL = 6
# 1980-01-01, the earliest timestamp a zip can encode (upstream's default).
DOS_EPOCH = 315532800

PLUGIN_NAME_RE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
WASM_PATH_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*\.wasm$")
SEMVER_NUMERIC_IDENTIFIER = r"(?:0|[1-9][0-9]*)"
SEMVER_PRERELEASE_IDENTIFIER = (
    rf"(?:{SEMVER_NUMERIC_IDENTIFIER}|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
)
SEMVER_BUILD_IDENTIFIER = r"[0-9A-Za-z-]+"
PLUGIN_VERSION_RE = re.compile(
    rf"^{SEMVER_NUMERIC_IDENTIFIER}"
    rf"\.{SEMVER_NUMERIC_IDENTIFIER}"
    rf"\.{SEMVER_NUMERIC_IDENTIFIER}"
    rf"(?:-{SEMVER_PRERELEASE_IDENTIFIER}"
    rf"(?:\.{SEMVER_PRERELEASE_IDENTIFIER})*)?"
    rf"(?:\+{SEMVER_BUILD_IDENTIFIER}(?:\.{SEMVER_BUILD_IDENTIFIER})*)?$"
)

KNOWN_CAPABILITIES = {"tool", "channel", "memory", "observer", "skill"}
KNOWN_PERMISSIONS = {
    "http_client",
    "file_read",
    "file_write",
    "config_read",
    "env_read",  # serde alias for config_read
    "memory_read",
    "memory_write",
    "socket_client",
    "websocket_client",
}
# Accepted only by a `registry = false` plugin until a host release ships them.
HOST_GATED_PERMISSIONS = {"state_read", "state_write"}
KNOWN_SENDER_MATCH = {"exact", "case_insensitive", "handle", "email"}

# manifest.toml is canonical; these fields are projected into registry entries.
REGISTRY_METADATA_FIELDS = (
    "name",
    "version",
    "description",
    "author",
    "capabilities",
    "provides",
    "sender_match",
)
REGISTRY_RELEASE_FIELDS = ("url", "sha256")

DEFAULT_UPSTREAM_REGISTRY = (
    "https://raw.githubusercontent.com/zeroclaw-labs/zeroclaw-plugins/main/registry.json"
)
DEFAULT_RELEASE_BASE = (
    "https://github.com/seungjin/hanaclaw-plugins/releases/download/" + PLUGIN_RELEASE_TAG
)


class PackageError(Exception):
    """A fatal input problem; reported without a traceback."""


# ── versions and ordering ──────────────────────────────────────────────────


def version_key(version: str) -> tuple:
    """Sort key for SemVer precedence; a release sorts after its prereleases."""
    core, _, _build = version.partition("+")
    release, _, prerelease = core.partition("-")
    numbers = tuple(int(part) for part in release.split("."))
    if not prerelease:
        return (numbers, (1,), ())
    parts = []
    for ident in prerelease.split("."):
        parts.append((0, int(ident), "") if ident.isdigit() else (1, 0, ident))
    return (numbers, (0,), tuple(parts))


def entry_key(entry: dict) -> tuple:
    return (entry["name"], version_key(entry["version"]))


# ── manifest ───────────────────────────────────────────────────────────────


@dataclasses.dataclass(frozen=True)
class Plugin:
    """One source plugin directory and its parsed manifest."""

    dir: Path
    manifest: dict

    @property
    def name(self) -> str:
        return self.manifest["name"]

    @property
    def version(self) -> str:
        return self.manifest["version"]

    @property
    def identity(self) -> tuple[str, str]:
        return (self.name, self.version)

    @property
    def wasm_path(self) -> str:
        return self.manifest["wasm_path"]

    @property
    def publishable(self) -> bool:
        return self.manifest.get("registry", True) is not False


def read_manifest(pdir: Path) -> dict:
    path = pdir / "manifest.toml"
    try:
        return tomllib.loads(path.read_text())
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as e:
        raise PackageError(f"{path}: {e}") from e


def _strings(meta: dict, field: str, errors: list[str]) -> list[str]:
    value = meta.get(field, [])
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        errors.append(f"{field} must be an array of strings")
        return []
    return value


def validate(plugin: Plugin) -> list[str]:
    """Check a plugin against the host's install contract.

    Mirrors upstream ``build-registry.py validate(require_wasm=False)``, which
    is strict about everything except the built .wasm.
    """
    meta = plugin.manifest
    pdir = plugin.dir
    errors: list[str] = []

    name = meta.get("name")
    if not isinstance(name, str) or not PLUGIN_NAME_RE.fullmatch(name):
        errors.append(f"name {name!r} must be a lowercase slug ([a-z0-9][a-z0-9_-]*)")
    elif name != pdir.name:
        errors.append(f"name {name!r} must match directory {pdir.name!r}")

    version = meta.get("version")
    if not isinstance(version, str) or not PLUGIN_VERSION_RE.fullmatch(version):
        errors.append(f"version {version!r} must be a semantic version")

    cargo = pdir / "Cargo.toml"
    if cargo.is_file():
        try:
            cargo_version = tomllib.loads(cargo.read_text())["package"]["version"]
        except (OSError, UnicodeDecodeError, KeyError, tomllib.TOMLDecodeError) as e:
            errors.append(f"cannot validate Cargo.toml package.version: {e!r}")
        else:
            if cargo_version != version:
                errors.append(
                    f"Cargo.toml package.version {cargo_version!r} does not match "
                    f"manifest version {version!r}"
                )

    for field in ("description", "author", "provides", "sender_match"):
        value = meta.get(field)
        if value is not None and (not isinstance(value, str) or not value.strip()):
            errors.append(f"{field} must be a non-empty string when present")
    sender_match = meta.get("sender_match")
    if isinstance(sender_match, str) and sender_match not in KNOWN_SENDER_MATCH:
        errors.append(
            f"unknown sender_match {sender_match!r}; expected one of "
            f"{sorted(KNOWN_SENDER_MATCH)!r}"
        )

    capabilities = _strings(meta, "capabilities", errors)
    if not capabilities:
        errors.append("capabilities must be a non-empty array")
    errors.extend(
        f"unknown capability {cap!r} (the host rejects the manifest)"
        for cap in sorted(set(capabilities) - KNOWN_CAPABILITIES)
    )

    registry = meta.get("registry", True)
    if not isinstance(registry, bool):
        errors.append("registry must be a boolean when present")
    permissions = _strings(meta, "permissions", errors)
    errors.extend(
        f"unknown permission {perm!r} (the host rejects the manifest)"
        for perm in sorted(set(permissions) - KNOWN_PERMISSIONS - HOST_GATED_PERMISSIONS)
    )
    if registry is not False:
        errors.extend(
            f"permission {perm!r} is rejected by released hosts; "
            "set registry = false until a host release accepts it"
            for perm in sorted(set(permissions) & HOST_GATED_PERMISSIONS)
        )

    wasm_path = meta.get("wasm_path")
    needs_wasm = bool(set(capabilities) - {"skill"})
    if needs_wasm and wasm_path is None:
        errors.append("wasm_path is required for non-skill-only plugins")
    elif wasm_path is not None and not (
        isinstance(wasm_path, str) and WASM_PATH_RE.fullmatch(wasm_path)
    ):
        errors.append("wasm_path must be a safe relative .wasm filename")
    return errors


def load_source_plugins(plugins_root: Path) -> tuple[list[Plugin], list[str]]:
    """Load and validate ``plugins/*/manifest.toml`` (a .wasm is not required)."""
    if not plugins_root.is_dir():
        raise PackageError(f"plugins directory not found: {plugins_root}")
    plugins: list[Plugin] = []
    errors: list[str] = []
    for pdir in sorted(plugins_root.iterdir()):
        if pdir.is_symlink():
            errors.append(f"{pdir.name}: plugin directory must not be a symbolic link")
            continue
        if not pdir.is_dir() or not (pdir / "manifest.toml").is_file():
            continue
        plugin = Plugin(dir=pdir, manifest=read_manifest(pdir))
        errors.extend(f"{pdir.name}: {e}" for e in validate(plugin))
        plugins.append(plugin)
    return plugins, errors


# ── reproducible package zip ───────────────────────────────────────────────


def zip_date_time() -> tuple[int, int, int, int, int, int]:
    """Fixed timestamp (SOURCE_DATE_EPOCH, else 1980-01-01) in DOS 2s units."""
    epoch = max(int(os.environ.get("SOURCE_DATE_EPOCH", DOS_EPOCH)), DOS_EPOCH)
    stamp = time.gmtime(epoch)
    return (
        stamp.tm_year,
        stamp.tm_mon,
        stamp.tm_mday,
        stamp.tm_hour,
        stamp.tm_min,
        stamp.tm_sec - stamp.tm_sec % 2,
    )


def zlib_is_ng() -> bool:
    return "zlib-ng" in getattr(zlib, "ZLIB_RUNTIME_VERSION", "")


def warn_if_zlib_ng(stream=sys.stderr) -> None:
    """A zlib-ng build compresses differently, so every digest would differ."""
    if zlib_is_ng():
        print(
            "warning: this Python links zlib-ng, whose deflate output differs "
            "from stock zlib; package digests will not match stock-zlib builds. "
            "Use `uv run --python 3.13 python ...`.",
            file=stream,
        )


def package_wasm(pdir: Path, wasm_path: str) -> bytes:
    """The built component's bytes, validated on their own.

    Kept apart from ``package_files`` so CI can hash the wasm a shard produced
    before dropping it next to ``manifest.toml`` (the only path the
    packaging step can read).
    """
    return read_wasm(pdir / wasm_path)


def read_wasm(wasm: Path) -> bytes:
    """The bytes of a built component, refusing links and empty files."""
    if wasm.is_symlink() or not wasm.is_file():
        raise PackageError(f"{wasm}: missing or not a regular file")
    contents = wasm.read_bytes()
    if not contents:
        raise PackageError(f"{wasm}: empty")
    return contents


def package_files(pdir: Path, wasm_path: str) -> dict[str, bytes]:
    """The two files the host installs: manifest.toml and the .wasm."""
    files = {"manifest.toml": (pdir / "manifest.toml").read_bytes()}
    for relative in files:
        if (pdir / relative).is_symlink():
            raise PackageError(f"{pdir / relative}: symbolic links are not allowed")
    files[wasm_path] = package_wasm(pdir, wasm_path)
    extracted = sum(len(contents) for contents in files.values())
    if extracted > MAX_PLUGIN_EXTRACTED_BYTES:
        raise PackageError(
            f"{pdir.name}: extracted size {extracted} exceeds the host's "
            f"{MAX_PLUGIN_EXTRACTED_BYTES}-byte install cap"
        )
    return files


def _write_zip(archive: zipfile.ZipFile, name: str, files: dict[str, bytes]) -> None:
    date_time = zip_date_time()
    for relative in sorted(files):
        info = zipfile.ZipInfo(f"{name}/{relative}", date_time=date_time)
        info.create_system = 3
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = 0o644 << 16
        archive.writestr(
            info,
            files[relative],
            compress_type=zipfile.ZIP_DEFLATED,
            compresslevel=ZIP_COMPRESSION_LEVEL,
        )


def write_zip(path: Path, name: str, files: dict[str, bytes]) -> None:
    """Write the package zip exactly as upstream ``write_zip`` does.

    Sorted ``<name>/`` entries, fixed timestamps, mode 0644, Unix creator,
    deflate at level 6: identical content always yields identical bytes.
    """
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        # Not implied by the mode above: without it the level is the
        # interpreter's default, and a future default would silently change
        # every digest this repo has published.
        archive.compresslevel = ZIP_COMPRESSION_LEVEL
        _write_zip(archive, name, files)


def zip_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def package_sha256(pdir: Path, name: str, wasm_path: str) -> str:
    """sha256 of the package zip, without writing it to the repository."""
    files = package_files(pdir, wasm_path)
    with tempfile.TemporaryFile() as buffer, zipfile.ZipFile(
        buffer, "w", zipfile.ZIP_DEFLATED
    ) as archive:
        _write_zip(archive, name, files)
        archive.close()
        buffer.seek(0)
        digest = hashlib.sha256()
        for chunk in iter(lambda: buffer.read(1 << 16), b""):
            digest.update(chunk)
    return digest.hexdigest()


# ── registry document ──────────────────────────────────────────────────────


def validate_registry(registry: object, *, source: str) -> None:
    """Structural checks on a fetched or local registry document."""
    if not isinstance(registry, dict) or not isinstance(registry.get("plugins"), list):
        raise PackageError(f"{source}: expected an object with a 'plugins' array")
    seen: set[tuple[str, str]] = set()
    for index, entry in enumerate(registry["plugins"]):
        where = f"{source}: plugins[{index}]"
        if not isinstance(entry, dict):
            raise PackageError(f"{where}: entry must be an object")
        for field in ("name", "version", "description", "author", "url", "sha256"):
            if not isinstance(entry.get(field), str) or not entry[field]:
                raise PackageError(f"{where}: missing or non-string '{field}'")
        if not PLUGIN_NAME_RE.match(entry["name"]):
            raise PackageError(f"{where}: invalid plugin name {entry['name']!r}")
        if not PLUGIN_VERSION_RE.match(entry["version"]):
            raise PackageError(f"{where}: invalid version {entry['version']!r}")
        if not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]):
            raise PackageError(f"{where}: sha256 is not 64 lowercase hex digits")
        capabilities = entry.get("capabilities")
        if not isinstance(capabilities, list) or not all(
            isinstance(c, str) for c in capabilities
        ):
            raise PackageError(f"{where}: 'capabilities' must be an array of strings")
        identity = (entry["name"], entry["version"])
        # Published identities are immutable: a duplicate would make installs
        # ambiguous depending on which copy the host resolves.
        if identity in seen:
            raise PackageError(f"{where}: duplicate {identity[0]}@{identity[1]}")
        seen.add(identity)


def fetch_upstream(url: str, timeout: float = 30) -> dict:
    token = os.environ.get("GITHUB_TOKEN")
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": "hanaclaw-registry-updater",
            **({"Authorization": f"Bearer {token}"} if token else {}),
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
    except urllib.error.HTTPError as e:
        raise PackageError(f"fetching {url}: HTTP {e.code} {e.reason}") from e
    except (urllib.error.URLError, TimeoutError) as e:
        raise PackageError(f"fetching {url}: {e}") from e
    try:
        registry = json.loads(raw)
    except json.JSONDecodeError as e:
        raise PackageError(f"{url} is not valid JSON: {e}") from e
    validate_registry(registry, source=url)
    return registry


def manifest_metadata(meta: dict) -> dict:
    """Project canonical manifest fields into one registry metadata view."""
    return {
        key: meta[key]
        for key in REGISTRY_METADATA_FIELDS
        if key in meta and meta[key] is not None
    }


def registry_metadata_view(entry: dict) -> dict:
    return {
        key: entry[key]
        for key in REGISTRY_METADATA_FIELDS
        if key in entry and entry[key] is not None
    }


def load_registry(path: Path) -> tuple[dict, dict[tuple[str, str], dict]]:
    try:
        registry = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as e:
        raise PackageError(f"cannot read registry {path}: {e}") from e
    validate_registry(registry, source=str(path))
    by_identity = {(e["name"], e["version"]): e for e in registry["plugins"]}
    return registry, by_identity


def render(registry: dict) -> str:
    return json.dumps(registry, indent=2, ensure_ascii=False) + "\n"
