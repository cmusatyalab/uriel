#!/bin/bash
# Builds a release's files for one architecture, in a manylinux_2_28 container, so that they work
# on any Linux with glibc 2.28 or newer:
#
#   dist/lib/liburiel-<version>-linux-<arch>.tar.gz   the static library, header and uriel.pc
#   dist/python/uriel-<version>-*.whl                  the Python wheel
#   dist/python/uriel-<version>.tar.gz                 the sdist, when SDIST is true
#
# .github/workflows/release.yml runs it from the repository's root:
#
#   docker run --rm -v "$PWD:/src" -w /src -e VERSION -e ARCH -e RUST_VERSION -e CARGO_C_VERSION \
#       -e SDIST quay.io/pypa/manylinux_2_28_<arch> .github/scripts/build-release.sh
set -euo pipefail

: "${VERSION:?}" "${ARCH:?}" "${RUST_VERSION:?}" "${CARGO_C_VERSION:?}"
SDIST=${SDIST:-false}
# Any of the image's Pythons will do: the wheel works with every Python 3.
PYTHON=/opt/python/cp312-cp312/bin/python
NAME="liburiel-$VERSION-linux-$ARCH"

# libclang generates the bindings of iceoryx2, Uriel's shared memory transport.
dnf install -y --quiet clang-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --quiet --profile minimal --default-toolchain "$RUST_VERSION"
export PATH="$HOME/.cargo/bin:$PATH"
# cargo-c's release is a static binary.
curl --proto '=https' --tlsv1.2 -sSfL \
    "https://github.com/lu-zero/cargo-c/releases/download/v$CARGO_C_VERSION/cargo-c-$ARCH-unknown-linux-musl.tar.gz" \
    | tar -xz -C "$HOME/.cargo/bin"
"$PYTHON" -m pip install --quiet uv
# The checkout belongs to the runner's user, not to the container's root.
git config --global --add safe.directory "$PWD"

mkdir -p dist/lib dist/python
stage=$(mktemp -d)

# Only the static library, so a program linked with it needs nothing of Uriel's where it runs.
cargo cinstall --release --locked -p uriel --library-type staticlib \
    --prefix /usr/local --libdir lib --destdir "$stage"
mv "$stage/usr/local" "$stage/$NAME"
strip --strip-debug "$stage/$NAME/lib/liburiel.a"
# cargo-c names the install prefix in uriel.pc; name the directory the file is in instead, so
# the tarball works wherever it is unpacked.
sed -i 's|^prefix=.*|prefix=${pcfiledir}/../..|' "$stage/$NAME/lib/pkgconfig/uriel.pc"
cp LICENSE "$stage/$NAME/"
tar -czf "dist/lib/$NAME.tar.gz" -C "$stage" "$NAME"

cd python
# Tag the wheel manylinux_2_28 and check that it is: uv asks maturin for no tag (linux_<arch>),
# which PyPI does not accept.
export MATURIN_PEP517_ARGS="--compatibility manylinux_2_28"
uv build --python "$PYTHON" --wheel --out-dir ../dist/python
if [ "$SDIST" = true ]; then
    uv build --python "$PYTHON" --sdist --out-dir ../dist/python
fi
# uv leaves a .gitignore in the directory, which is not part of the release.
rm -f ../dist/python/.gitignore
