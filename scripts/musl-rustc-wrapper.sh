#!/bin/sh
# rustc wrapper for musl (Alpine) builds of the extension. CMakeLists.txt sets
# it as RUSTC_WRAPPER for the cargo build when the Rust target is *-musl.
#
# The rustup musl toolchain links binaries statically by default (crt-static),
# which breaks the build scripts of custom-labels and vortex-duckdb: they run
# bindgen, which dlopen()s libclang, and a static musl binary cannot dlopen
# ("Dynamic loading not supported"). The Rust staticlib also has to be position
# independent because it is folded into the extension's shared object.
#
# Corrosion always passes `--target` to cargo, and under `--target` cargo only
# forwards RUSTFLAGS (env, `build.rustflags` or `[target.*].rustflags`) to
# target artifacts, never to host build scripts. A RUSTC_WRAPPER sees every
# rustc invocation, host and target alike, so it is the stable mechanism that
# reaches the build scripts.
#
# Cargo also runs probes such as `rustc -vV` through the wrapper; only real
# compilations (and cargo's target-info probe) carry `--crate-name`.
set -eu
for arg in "$@"; do
    if [ "$arg" = "--crate-name" ]; then
        set -- "$@" -C target-feature=-crt-static -C relocation-model=pic
        break
    fi
done
exec "$@"
