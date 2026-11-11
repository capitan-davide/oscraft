#!/bin/sh
# Checks what building and testing OsCraft needs on this host, and says
# what is missing: `make doctor`. The QNX SDP is optional (QNX=1, the
# qnx-* targets, the probe, traces and dumps): its section only notes.
here=$(cd "$(dirname "$0")/.." && pwd)
ok=0; bad=0
have() { if command -v "$1" >/dev/null 2>&1; then printf 'ok   %s\n' "$2"; ok=$((ok+1)); else printf 'MISSING %s (%s)\n' "$2" "$1"; bad=$((bad+1)); fi; }
check() { if eval "$2" >/dev/null 2>&1; then printf 'ok   %s\n' "$1"; ok=$((ok+1)); else printf 'MISSING %s\n' "$1"; bad=$((bad+1)); fi; }
note() { if eval "$2" >/dev/null 2>&1; then printf 'ok   %s\n' "$1"; else printf 'note %s: not set (optional)\n' "$1"; fi; }

echo "# host tools"
have clang "clang (FreeBSD's userland and the C tests)"
have ld.lld "ld.lld"
have mkfs.fat "mkfs.fat (a FAT image for the tests, dosfstools)"
have mcopy "mcopy (a FAT image for the tests, mtools)"
have qemu-system-aarch64 "qemu-system-aarch64 (make run, make test)"
have python3 "python3 (the test runner, mkifs tools, FreeBSD's make.py)"
have git git
have tic "tic (the terminfo entries, make freebsd-terminfo)"
have ssh-keygen "ssh-keygen (sshd's host key in the image)"
if command -v bc >/dev/null 2>&1; then echo "ok   bc (bmake's self-test)"; else echo "note bc absent: bmake's self-test cannot pass; the bootstrap takes the build as tested"; fi
echo "# Rust"
have cargo cargo
check "the kernel's toolchain ($(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$here/rust-toolchain.toml"))" "cargo +$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$here/rust-toolchain.toml") --version"
check "the target aarch64-unknown-none-softfloat" "rustup target list --installed | grep -q aarch64-unknown-none-softfloat"
LIBSYS_TC=$(sed -n 's/^LIBSYS_TOOLCHAIN ?= *//p' "$here/Makefile")
check "libsys's toolchain $LIBSYS_TC with rust-src (-Zbuild-std)" "cargo +$LIBSYS_TC --version && rustup component list --toolchain $LIBSYS_TC --installed | grep -q rust-src"
echo "# FreeBSD's source and build (tools/freebsd/bootstrap.sh)"
SRC=${FREEBSD_SRC:-$here/../freebsd-src}
OBJ=${FREEBSD_OBJ:-$here/../freebsd-obj}
check "the tree at $SRC on branch oscraft" "git -C '$SRC' rev-parse --verify -q oscraft"
check "its patches applied ($(ls "$here"/tools/freebsd/patches | wc -l))" "[ \$(git -C '$SRC' log --oneline \$(cat '$here/tools/freebsd/UPSTREAM_COMMIT')..oscraft | wc -l) -ge \$(ls '$here'/tools/freebsd/patches | wc -l) ]"
check "bmake bootstrapped in $OBJ" "[ -x '$OBJ/bmake-install/bin/bmake' ]"
check "the sysroot's headers ($OBJ$SRC/arm64.aarch64/tmp)" "[ -d '$OBJ$SRC/arm64.aarch64/tmp/usr/include/sys' ]"
check "the sysroot's libc (make freebsd-libs)" "[ -f '$OBJ$SRC/arm64.aarch64/tmp/usr/lib/libc.a' ]"
check "FreeBSD's sh built (make freebsd-progs)" "[ -x '$OBJ$SRC/arm64.aarch64/tmp/oscraft/bin/sh' ]"
echo "# QNX SDP 8.0, optional (the user's licence; source its qnxsdp-env.sh in bash for QNX=1)"
note "QNX_HOST set and present" '[ -n "$QNX_HOST" ] && [ -d "$QNX_HOST" ]'
note "QNX_TARGET set and present" '[ -n "$QNX_TARGET" ] && [ -d "$QNX_TARGET" ]'
echo "# $ok ok, $bad missing"
[ "$bad" -eq 0 ]
