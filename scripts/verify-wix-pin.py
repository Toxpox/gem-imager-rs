#!/usr/bin/env python3
"""Check the WIX constants in release.yml against the pinned cargo-packager release.

The release workflow prefetches the WIX toolset itself so a transient network error retries
instead of aborting the build. Doing that means restating cargo-packager's own download URL,
checksum and expected file list. If cargo-packager ever bumps the toolset, the prefetched
directory stops satisfying it and the fallback path this step exists to avoid comes back,
silently and only on a tagged release.

Read the pin out of the Makefile, fetch that exact release from crates.io and compare.
"""
import io
import re
import ssl
import sys
import tarfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAKEFILE = ROOT / "Makefile"
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
CRATE_URL = "https://static.crates.io/crates/cargo-packager/cargo-packager-{v}.crate"
WIX_SRC = "cargo-packager-{v}/src/package/wix/mod.rs"


def fail(msg: str) -> None:
    sys.exit(f"error: {msg}")


def pinned_version() -> str:
    text = MAKEFILE.read_text()
    m = re.search(r"install cargo-packager --locked --version (\S+)", text)
    if not m:
        fail("no pinned cargo-packager version found in the Makefile")
    return m.group(1)


def fetch(url: str, attempts: int = 4) -> bytes:
    last = None
    for attempt in range(1, attempts + 1):
        try:
            with urllib.request.urlopen(url, timeout=60) as resp:
                return resp.read()
        except (urllib.error.URLError, ssl.SSLError, TimeoutError) as err:
            last = err
            print(f"attempt {attempt} failed: {err}")
            if attempt < attempts:
                time.sleep(2**attempt)
    fail(f"could not fetch {url}: {last}")


def packager_wix_source(version: str) -> str:
    raw = fetch(CRATE_URL.format(v=version))
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as tar:
        member = tar.extractfile(WIX_SRC.format(v=version))
        if member is None:
            fail(f"{WIX_SRC.format(v=version)} is missing from the crate archive")
        return member.read().decode()


def packager_constants(src: str) -> tuple[str, str, list[str]]:
    url = re.search(r'pub const WIX_URL: &str =\s*"([^"]+)"', src)
    sha = re.search(r'pub const WIX_SHA256: &str = "([0-9a-f]+)"', src)
    files = re.search(r"const WIX_REQUIRED_FILES: &\[&str\] = &\[(.*?)\];", src, re.S)
    if not (url and sha and files):
        fail("cargo-packager no longer declares WIX_URL / WIX_SHA256 / WIX_REQUIRED_FILES")
    return url.group(1), sha.group(1), re.findall(r'"([^"]+)"', files.group(1))


def workflow_constants() -> tuple[str, str, list[str]]:
    text = WORKFLOW.read_text()
    url = re.search(r"\$url = '([^']+)'", text)
    sha = re.search(r"\$expected = '([0-9a-f]+)'", text)
    files = re.search(r"\$required = @\((.*?)\)", text, re.S)
    if not (url and sha and files):
        fail("the prefetch step in release.yml no longer declares $url / $expected / $required")
    return url.group(1), sha.group(1), re.findall(r"'([^']+)'", files.group(1))


def main() -> None:
    version = pinned_version()
    print(f"pinned cargo-packager: {version}")

    want_url, want_sha, want_files = packager_constants(packager_wix_source(version))
    got_url, got_sha, got_files = workflow_constants()

    problems = []
    if got_url != want_url:
        problems.append(f"url: workflow has {got_url}, cargo-packager uses {want_url}")
    if got_sha != want_sha:
        problems.append(f"sha256: workflow has {got_sha}, cargo-packager uses {want_sha}")
    if sorted(got_files) != sorted(want_files):
        missing = sorted(set(want_files) - set(got_files))
        extra = sorted(set(got_files) - set(want_files))
        problems.append(f"required files differ; missing={missing} unexpected={extra}")

    if problems:
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        fail(
            "the WIX prefetch step no longer matches cargo-packager "
            f"{version}; update .github/workflows/release.yml"
        )

    print(f"WIX prefetch matches cargo-packager {version}")
    print(f"  url    {got_url}")
    print(f"  sha256 {got_sha}")
    print(f"  files  {len(got_files)}")


if __name__ == "__main__":
    main()
