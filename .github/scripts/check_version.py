"""Checks that a release tag names the version of the Rust crate and of the Python package.

    uv run --no-project --with packaging python .github/scripts/check_version.py v0.1.0

The tag is v<version>, e.g. v0.1.0, or v0.1.0-rc1 for a release candidate: the Go module needs a
semantic version, and Python reads 0.1.0-rc1 as 0.1.0rc1. It prints the version, as the tag
spells it, for the release's file names.
"""

import sys
import tomllib
from pathlib import Path

from packaging.version import Version

ROOT = Path(__file__).resolve().parents[2]
TAG_PREFIX = "v"


def main() -> None:
    tag = sys.argv[1]
    if not tag.startswith(TAG_PREFIX):
        sys.exit(f"the tag {tag} does not start with {TAG_PREFIX}")
    version = tag.removeprefix(TAG_PREFIX)

    crate = tomllib.loads((ROOT / "rust/Cargo.toml").read_text())["package"]["version"]
    package = tomllib.loads((ROOT / "python/pyproject.toml").read_text())["project"]["version"]
    if crate != version:
        sys.exit(f"rust/Cargo.toml has version {crate}, but the tag is {tag}")
    if Version(package) != Version(version):
        sys.exit(f"python/pyproject.toml has version {package}, but the tag is {tag}")
    print(version)


if __name__ == "__main__":
    main()
