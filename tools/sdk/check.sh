#!/bin/sh
# Checks an assembled SDK from outside the build tree: a C program through
# the CMake toolchain file and a Rust program through the Cargo
# configuration, both built in a scratch directory with the host's tools,
# then run on OsCraft in QEMU (the image gets the two programs, the test
# runner looks for their lines).
#   tools/sdk/check.sh <sdk-dir> [nightly-toolchain]
set -eu
SDK=$(cd "$1" && pwd)
NIGHTLY=${2:-nightly}
here=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# ANCHOR: out_of_tree
echo "=== C through CMake"
cmake -S "$here/tools/sdk/check" -B "$work/c" -DCMAKE_TOOLCHAIN_FILE="$SDK/oscraft.cmake" >/dev/null
cmake --build "$work/c" >/dev/null
echo "=== Rust through Cargo ($NIGHTLY, std from source)"
cp -r "$here/tools/sdk/check/rust-hello" "$work/rust"
mkdir -p "$work/rust/.cargo"
cp "$SDK/cargo-config.toml" "$work/rust/.cargo/config.toml"
(cd "$work/rust" && PATH="$SDK/bin:$PATH" CARGO_TARGET_DIR="$work/rust/target" cargo "+$NIGHTLY" build --release -q)
RUST_BIN="$work/rust/target/aarch64-unknown-freebsd/release/sdk-rust"
# ANCHOR_END: out_of_tree
echo "=== on OsCraft"
cat > "$work/extra.build" <<EOT
/proc/boot/sdk-c=$work/c/sdk-c
/proc/boot/sdk-rust=$RUST_BIN
EOT
cat > "$work/sdk.tests" <<'EOT'
name: sdk-check
timeout: 20
run: sdk-c; sdk-rust a b
expect: ^sdk-c: hello from OsCraft on arm64, pid>1:1$
expect: ^sdk-rust: hello, 10! = 3628800, args=3$
EOT
(cd "$here" && make -s image IMAGE_EXTRA="$work/extra.build" >/dev/null && python3 -I tools/test/run-tests.py build/oscraft.ifs "$work/sdk.tests")
