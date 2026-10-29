#!/usr/bin/env python3
"""Vendor ZeroClaw's plugin WIT contract into ``wit/<version>/``.

Plugin components bind against a WIT ``world`` supplied by the host, so the
WIT tree in this repo must be byte-identical to the tree the target host was
built from. This copies

    https://github.com/zeroclaw-labs/zeroclaw-plugins/tree/main/wit

(from the ``zeroclaw`` source repo's ``wit/``, the tree the host actually
builds from) at one pinned commit, records that commit in
``wit/<version>/UPSTREAM_REF``, and writes ``wit/<version>/.sha256`` over the
vendored files so drift is detectable without network access.

Usage (from the repository root)::

    uv run --python 3.13 python tools/fetch_zeroclaw_wit.py              # fetch + install wit/v0
    uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --version next
    uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --check      # CI/offline drift check
    uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --diff       # show what would change
    uv run --python 3.13 python tools/fetch_zeroclaw_wit.py --ref <sha>  # pin a specific contract

The default ``--ref master`` is the moving tip of zeroclaw (whose default
branch is ``master``, unlike zeroclaw-plugins' ``main``); pin
``--ref <sha>`` when the copy must not drift. The
registry's ``wit/UPSTREAM_REF`` names a ZeroClaw commit, not a
zeroclaw-plugins one, so the registry's own ``wit/`` checkout is the
authoritative source for a released contract — pass it with
``--upstream-dir``.

Requires Python 3.11+. Standard library only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request
from pathlib import Path, PurePosixPath

DEFAULT_REPO = "zeroclaw-labs/zeroclaw"
# zeroclaw-plugins' default branch is `main`, but its wit/ tree is a vendored
# copy of this repo's, whose default branch is `master`.
DEFAULT_REF = "master"
# A contract directory is a plain name like v0/next/current/unstable. Reject
# anything that could escape wit/ or collide with the bookkeeping files.
VERSION_RE = re.compile(r"^[a-z][a-z0-9_-]*$")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
CHECKSUM_FILE = ".sha256"
PIN_FILE = "UPSTREAM_REF"


class WitError(Exception):
    """A fatal input problem; reported without a traceback."""


# ── upstream access ────────────────────────────────────────────────────────


def _github_auth() -> dict[str, str]:
    token = os.environ.get("GITHUB_TOKEN")
    return {"Authorization": f"Bearer {token}"} if token else {}


def _get(url: str, timeout: float) -> bytes:
    request = urllib.request.Request(
        url, headers={"User-Agent": "hanaclaw-wit-fetcher", **_github_auth()}
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.read()
    except urllib.error.HTTPError as e:
        raise WitError(f"fetching {url}: HTTP {e.code} {e.reason}") from e
    except (urllib.error.URLError, TimeoutError) as e:
        raise WitError(f"fetching {url}: {e}") from e


def resolve_ref(repo: str, ref: str, timeout: float = 30) -> str:
    """Full commit SHA for a branch, tag, or SHA (GitHub resolves all three)."""
    if SHA_RE.match(ref):
        return ref
    url = f"https://api.github.com/repos/{repo}/commits/{ref}"
    try:
        payload = json.loads(_get(url, timeout))
    except json.JSONDecodeError as e:
        raise WitError(f"{url} did not return JSON: {e}") from e
    sha = payload.get("sha") if isinstance(payload, dict) else None
    if not isinstance(sha, str) or not SHA_RE.match(sha):
        raise WitError(f"{url}: could not resolve {ref!r} to a commit")
    return sha


def download_tarball(repo: str, sha: str, timeout: float = 120) -> Path:
    """Download the source tarball for a commit into a temp dir."""
    url = f"https://codeload.github.com/{repo}/tar.gz/{sha}"
    data = _get(url, timeout)
    handle = tempfile.NamedTemporaryFile(suffix=".tar.gz", delete=False)
    handle.write(data)
    handle.close()
    return Path(handle.name)


def extract_wit(tarball: Path, sha: str, version: str) -> dict[str, bytes]:
    """Return the bytes of ``wit/<version>/`` for the commit.

    Upstream's ``wit/`` root holds ``VERSIONING.md`` and one directory per
    package major (``v0/``, ``next/``, …); only ``wit/<version>/`` is
    vendored, with paths relative to that directory. GitHub tarballs are
    rooted at ``<repo>-<sha>/``; anything else (a redirect page, a truncated
    download) is rejected rather than misread.
    """
    wanted = ("wit", version)
    prefix = None
    files: dict[str, bytes] = {}
    with tarfile.open(tarball, "r:gz") as archive:
        for member in archive.getmembers():
            parts = PurePosixPath(member.name).parts
            if prefix is None and len(parts) >= 2:
                prefix = parts[0]
            if (
                len(parts) < 4
                or parts[0] != prefix
                or tuple(parts[1:3]) != wanted
                or not member.isfile()
            ):
                continue
            relative = PurePosixPath(*parts[3:]).as_posix()
            source = archive.extractfile(member)
            if source is None:  # pragma: no cover - regular member
                raise WitError(f"{member.name}: not a readable file")
            files[relative] = source.read()
    if not files:
        raise WitError(f"no wit/{version}/ tree in the tarball for {sha}")
    return files


def read_local_wit(wit_root: Path) -> dict[str, bytes]:
    files: dict[str, bytes] = {}
    for path in sorted(wit_root.rglob("*")):
        if path.is_symlink():
            raise WitError(f"{path}: symbolic links are not allowed in wit/")
        if path.is_file():
            files[path.relative_to(wit_root).as_posix()] = path.read_bytes()
    return files


# ── comparison and checksums ───────────────────────────────────────────────


def checksum_manifest(files: dict[str, bytes]) -> str:
    """Deterministic ``sha256  path`` listing over vendored content."""
    lines = [
        f"{hashlib.sha256(files[path]).hexdigest()}  {path}"
        for path in sorted(files)
        if path not in {CHECKSUM_FILE, PIN_FILE}
    ]
    return "\n".join(lines) + "\n"


def diff_files(old: dict[str, bytes], new: dict[str, bytes]) -> list[str]:
    changes = []
    for path in sorted(set(old) | set(new)):
        if path not in old:
            changes.append(f"add    {path}")
        elif path not in new:
            changes.append(f"remove {path}")
        elif old[path] != new[path]:
            changes.append(f"change {path}")
    return changes


def verify_local(wit_dir: Path) -> list[str]:
    """Problems with a vendored directory, checked without network access."""
    if not wit_dir.is_dir():
        return [f"{wit_dir}: not found (run tools/fetch_zeroclaw_wit.py)"]
    files = read_local_wit(wit_dir)
    pin = files.get(PIN_FILE, b"").decode("utf-8", "replace").strip()
    problems = []
    if not SHA_RE.match(pin):
        problems.append(f"{wit_dir / PIN_FILE}: expected one lowercase 40-hex commit SHA")
    recorded = files.get(CHECKSUM_FILE, b"").decode("utf-8", "replace")
    if not recorded:
        problems.append(f"{wit_dir / CHECKSUM_FILE}: missing (run the fetcher to create it)")
    elif recorded != checksum_manifest(files):
        problems.append(f"{wit_dir}: files differ from {CHECKSUM_FILE}; re-run the fetcher")
    return problems


# ── install ────────────────────────────────────────────────────────────────


def install(wit_dir: Path, files: dict[str, bytes], sha: str) -> None:
    """Replace ``wit_dir`` atomically with ``files`` plus pin and checksum.

    Only files that changed are rewritten, and files removed upstream are
    deleted, so a diff shows exactly the contract change.
    """
    staged = dict(files)
    staged[PIN_FILE] = f"{sha}\n".encode()
    staged[CHECKSUM_FILE] = checksum_manifest(staged).encode()

    wit_dir.mkdir(parents=True, exist_ok=True)
    for relative, contents in staged.items():
        target = wit_dir / relative
        if not target.parent.is_dir():
            target.parent.mkdir(parents=True)
        if target.is_symlink():
            raise WitError(f"{target}: refusing to overwrite a symbolic link")
        if target.is_file() and target.read_bytes() == contents:
            continue
        # Not `<name>.tmp`: rglob() below would treat a leftover as stray.
        tmp = target.with_name(f".{target.name}.partial")
        tmp.write_bytes(contents)
        os.replace(tmp, target)

    managed = {CHECKSUM_FILE, PIN_FILE} | set(files)
    for path in wit_dir.rglob("*"):
        if path.is_symlink():
            raise WitError(f"{path}: symbolic links are not allowed in wit/")
        if path.is_file() and path.relative_to(wit_dir).as_posix() not in managed:
            path.unlink()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Vendor the ZeroClaw plugin WIT contract into wit/<version>/."
    )
    parser.add_argument(
        "--repo", default=DEFAULT_REPO, help=f"owner/repo (default: {DEFAULT_REPO})"
    )
    parser.add_argument("--ref", default=DEFAULT_REF, help="branch, tag, or 40-hex SHA")
    parser.add_argument(
        "--version", default="v0", help="contract directory under wit/ (default: v0)"
    )
    parser.add_argument(
        "--wit-root", type=Path, default=Path("wit"), help="destination root (default: wit)"
    )
    parser.add_argument(
        "--upstream-dir",
        type=Path,
        help="read the upstream from a local checkout's wit/ tree instead of downloading",
    )
    parser.add_argument("--check", action="store_true", help="offline: verify the vendored copy")
    parser.add_argument("--diff", action="store_true", help="show changes without writing")
    parser.add_argument("--timeout", type=float, default=120, help="HTTP timeout in seconds")
    args = parser.parse_args(argv)

    try:
        if not VERSION_RE.fullmatch(args.version):
            raise WitError(f"--version must match {VERSION_RE.pattern}")
        wit_dir = args.wit_root / args.version

        if args.check:
            problems = verify_local(wit_dir)
            if problems:
                for problem in problems:
                    print(f"error: {problem}", file=sys.stderr)
                return 1
            print(f"{wit_dir} matches its {CHECKSUM_FILE} and {PIN_FILE}")
            return 0

        if args.upstream_dir is not None:
            source = args.upstream_dir / "wit" / args.version
            if not source.is_dir():
                source = args.upstream_dir  # caller pointed straight at wit/<version>
            files = read_local_wit(source)
            sha = "local"
            if not files:
                raise WitError(f"{source}: no WIT files")
        else:
            sha = resolve_ref(args.repo, args.ref, args.timeout)
            tarball = download_tarball(args.repo, sha, args.timeout)
            try:
                files = extract_wit(tarball, sha, args.version)
            finally:
                tarball.unlink(missing_ok=True)

        old = read_local_wit(wit_dir) if wit_dir.is_dir() else {}
        changes = diff_files(
            {k: v for k, v in old.items() if k not in {CHECKSUM_FILE, PIN_FILE}}, files
        )
        if args.diff:
            if not changes:
                print(f"{wit_dir} is identical to {args.repo} wit/ at {sha}")
            else:
                print(f"{wit_dir} differs from {args.repo} wit/ at {sha}:")
                for change in changes:
                    print(f"  {change}")
            return 0

        if not changes and sha != "local":
            pin = old.get(PIN_FILE, b"").decode().strip()
            if pin != sha:
                print(f"note: content unchanged; advancing {PIN_FILE} to {sha}")
        install(wit_dir, files, sha)
        total = len(changes)
        print(
            f"vendored {args.repo}/wit into {wit_dir} at {sha} "
            f"({len(files)} files, {total} changed)"
        )
        return 0
    except (WitError, OSError, tarfile.TarError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
