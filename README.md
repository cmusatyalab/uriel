# Uriel

Uriel moves data, such as video frames, from the apps that produce it to the engines that process it, and the engines' results back to the apps. Each app's inputs go through a flow of engines: on the same machine, through shared memory, or to a server on the network. Flow control keeps the newest input moving, so a slow engine never builds up a queue.

Uriel is written in Rust and has bindings for Python, Go and C. It runs on Linux.

## Install

### Python

```sh
pip install uriel
```

This needs Python 3.11 or newer, and Linux on x86_64 or aarch64 with glibc 2.28 or newer.

### Go and C

Go and C programs link liburiel, a static library. Each [release](https://github.com/cmusatyalab/uriel/releases) has it for x86_64 and aarch64, as `liburiel-<version>-linux-<arch>.tar.gz`. Use the library of the same version as the Go module.

**1. Download and unpack it.** Pick the architecture the program will run on:

```sh
curl -LO https://github.com/cmusatyalab/uriel/releases/download/v0.1.0/liburiel-0.1.0-linux-x86_64.tar.gz
tar -xzf liburiel-0.1.0-linux-x86_64.tar.gz
```

That gives:

```
liburiel-0.1.0-linux-x86_64/
├── include/uriel/uriel.h
├── lib/liburiel.a
├── lib/pkgconfig/uriel.pc
└── LICENSE
```

**2. Point pkg-config at it.** Either leave the directory where it is, anywhere you like, and set:

```sh
export PKG_CONFIG_PATH=$PWD/liburiel-0.1.0-linux-x86_64/lib/pkgconfig
```

or install it for the whole system, where pkg-config finds it by itself:

```sh
sudo tar -xzf liburiel-0.1.0-linux-x86_64.tar.gz -C /usr/local --strip-components=1
```

`pkg-config --libs uriel` should now print `-L…/lib -luriel …`.

**3. Build your program.**

In Go:

```sh
go get github.com/cmusatyalab/uriel/go@v0.1.0
```

```go
import "github.com/cmusatyalab/uriel/go" // package uriel
```

`go build` finds liburiel through pkg-config and links it into the binary. If the installed library has a different ABI version than the Go module, the build stops and says so.

In C:

```c
#include <uriel/uriel.h>
```

```sh
cc app.c $(pkg-config --cflags --libs uriel) -o app
```

CMake (`pkg_check_modules(URIEL REQUIRED uriel)`) and Meson (`dependency('uriel')`) find it through pkg-config too.

**Running the program.** It needs nothing of Uriel's: the program contains all of it, and only uses the system libraries that every Linux has.

**Cross-compiling,** e.g. for an arm64 device: use the aarch64 tarball and the cross compiler.

```sh
export PKG_CONFIG_PATH=$PWD/liburiel-0.1.0-linux-aarch64/lib/pkgconfig
CGO_ENABLED=1 GOOS=linux GOARCH=arm64 CC=aarch64-linux-gnu-gcc go build ./...
```

### From source

Building liburiel needs Rust, libclang, pkg-config and [cargo-c](https://github.com/lu-zero/cargo-c) (`cargo install cargo-c`). From a checkout:

```sh
cargo cinstall --release -p uriel --library-type staticlib --prefix ~/.local --libdir lib
export PKG_CONFIG_PATH=~/.local/lib/pkgconfig
```

Install it only as a static library, as above, so programs linked with it need nothing of Uriel's where they run.

## Examples

- `examples/go/hello` and `examples/python/hello.py`: an app sends text to an engine that upper-cases it.
- `examples/go/pipeline`: a camera, a motion filter and a detector in one process.
- `examples/yolo`: object detection with YOLO, in Python.
- `examples/h264`: engines that encode frames with H.264, and decode them.

## Developing

| Directory   | What's in it                                                        |
| ----------- | ------------------------------------------------------------------- |
| `rust/`     | The library, and its C ABI (`src/ffi.rs`, `include/uriel/uriel.h`). |
| `go/`       | The Go bindings, the module `github.com/cmusatyalab/uriel/go`.      |
| `python/`   | The Python bindings, the package `uriel`.                           |
| `server/`   | `uriel-server`, which serves a machine's engines to remote clients. |
| `examples/` | The examples above.                                                 |
| `bench/`    | Benchmarks.                                                         |

To build and test everything, install liburiel from source as above, and GStreamer's development files for `examples/h264` (`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on Ubuntu). Then:

```sh
cargo test --workspace                 # the Rust library; also regenerates the C header
(cd go && go test ./...)               # the Go bindings
(cd python && uv run pytest)           # the Python bindings
```

Install liburiel again whenever the Rust library changes, before testing the Go bindings.

### Releasing

The Rust crate and the Python package share one version. To release one:

1. Set it in `rust/Cargo.toml` (e.g. `0.2.0`, or `0.2.0-rc1` for a release candidate) and in `python/pyproject.toml` (`0.2.0`, or `0.2.0rc1`).
2. Update the lock files: `cargo update --workspace` and `(cd python && uv lock)`.
3. Commit, push, and push the tag `v0.2.0` (or `v0.2.0-rc1`).

The tag starts `.github/workflows/release.yml`. It publishes the Python package to PyPI (a release candidate to TestPyPI), puts liburiel and the Python files on a GitHub release, and tags the Go module `go/v0.2.0`.

## License

Apache 2.0. See [LICENSE](LICENSE).
