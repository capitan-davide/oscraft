#!/bin/sh
# Sets up FreeBSD's source and build system for OsCraft's userland, once
# per machine: OsCraft's fork of FreeBSD (github.com/capitan-davide/
# freebsd-src, branch oscraft: stable/15 at the pinned upstream commit
# plus the commits tools/freebsd/patches holds as patches), bmake bootstrapped
# as FreeBSD's tools/build/make.py wants it, and the cross-build's
# bootstrap stages, after which `make freebsd-libs` & co. work.
#
#   tools/freebsd/bootstrap.sh [src-dir] [obj-dir]
#
# Defaults: ../freebsd-src and ../freebsd-obj next to the repository
# (FREEBSD_SRC and FREEBSD_OBJ in the Makefile). Idempotent: a step
# already done is skipped. Needs: git, python3, clang, ld.lld, bmake's
# needs (a C compiler), and about 2 GB.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
SRC=${1:-$here/../freebsd-src}
OBJ=${2:-$here/../freebsd-obj}
UPSTREAM=$(cat "$here/tools/freebsd/UPSTREAM_COMMIT")
# The fork, with the branch; OSCRAFT_FREEBSD_GIT=https://git.freebsd.org/src.git
# clones upstream instead and applies the patches.
GIT_URL=${OSCRAFT_FREEBSD_GIT:-https://github.com/capitan-davide/freebsd-src.git}
JOBS=${JOBS:-$(nproc 2>/dev/null || echo 4)}

say() { printf '=== %s\n' "$*"; }

# 1. The source: a partial clone of the fork's branch `oscraft` (or of
# upstream's stable/15 at the pinned commit, with the patches applied on
# a branch `oscraft`).
if [ ! -d "$SRC/.git" ]; then
    say "cloning FreeBSD (blobs on demand) into $SRC"
    if git ls-remote --exit-code --heads "$GIT_URL" oscraft >/dev/null 2>&1; then
        git clone --filter=blob:none --branch oscraft "$GIT_URL" "$SRC"
    else
        git clone --filter=blob:none --branch stable/15 "$GIT_URL" "$SRC"
    fi
fi
if ! git -C "$SRC" rev-parse --verify -q oscraft >/dev/null; then
    say "checking out $UPSTREAM and applying OsCraft's patches"
    git -C "$SRC" fetch -q origin "$UPSTREAM" 2>/dev/null || true
    git -C "$SRC" checkout -q -b oscraft "$UPSTREAM"
    git -C "$SRC" am -q "$here"/tools/freebsd/patches/*.patch
else
    say "source: branch oscraft exists in $SRC ($(git -C "$SRC" rev-parse --short oscraft))"
fi

# 2. The host shims: a time(1) for hosts without it (FreeBSD's build
# times its stages).
mkdir -p "$OBJ/hostbin"
if [ ! -x "$OBJ/hostbin/time" ]; then
    cat > "$OBJ/hostbin/time" <<'SHIM'
#!/bin/sh
# time(1) shim for FreeBSD's build on a host without it: runs the command.
exec "$@"
SHIM
    chmod +x "$OBJ/hostbin/time"
fi

# 3. bmake, as make.py would bootstrap it, except that bmake's self-test
# fails on some hosts (it needs bc(1); one of its job tests fails under
# a parent make's environment): then the build is marked as tested by
# hand, and make.py's record of the configuration written so that it
# does not bootstrap again (its check compares the version and these
# arguments).
INSTALL="$OBJ/bmake-install"
if [ ! -x "$INSTALL/bin/bmake" ]; then
    say "bootstrapping bmake into $INSTALL"
    BUILD="$OBJ/bmake-build"
    rm -rf "$BUILD" "$INSTALL"
    mkdir -p "$BUILD"
    set -- --with-default-sys-path=".../share/mk:$INSTALL/share/mk" --with-machine=amd64 --without-filemon --prefix="$INSTALL"
    (cd "$BUILD" && sh "$SRC/contrib/bmake/boot-strap" op=build "$@") || {
        [ -x "$BUILD"/*/bmake ] || exit 1
        say "bmake built but its self-test failed (see above): taking it as tested"
        for d in "$BUILD"/*/; do touch "$d/tested"; done
    }
    (cd "$BUILD" && sh "$SRC/contrib/bmake/boot-strap" op=install "$@")
    # make.py's record, shell-quoted as it writes it (no quoting needed here).
    printf '%s' "--with-default-sys-path=.../share/mk:$INSTALL/share/mk --with-machine=amd64 --without-filemon --prefix=$INSTALL" > "$INSTALL/.make-py-config"
fi

# 4. The cross-build's stages, one at a time (bmake runs several
# command-line targets at once): the temporary tree, the legacy and
# bootstrap tools, the object tree, the build and cross tools, the
# metadata and the headers. Its temporary tree is the sysroot.
TMP="$OBJ$SRC/arm64.aarch64/tmp"
if [ ! -d "$TMP/usr/include/sys" ]; then
    say "FreeBSD's bootstrap stages (this takes a while)"
    for stage in _worldtmp _legacy _bootstrap-tools _obj _build-tools _cross-tools _build-metadata _includes; do
        say "stage $stage"
        (cd "$SRC" && env -u MAKEFLAGS -u MFLAGS -u MAKELEVEL MAKEOBJDIRPREFIX="$OBJ" PATH="$OBJ/hostbin:$PATH" \
            python3 tools/build/make.py --cross-bindir=/usr/bin TARGET=arm64 TARGET_ARCH=aarch64 \
            -j"$JOBS" -DWITHOUT_CLEAN -DWITHOUT_TESTS -DWITHOUT_PROFILE -DWITHOUT_MAN -DWITHOUT_LIB32 -DWITHOUT_DEBUG_FILES \
            DESTDIR="$TMP" "$stage")
    done
else
    say "stages: the sysroot exists at $TMP"
fi
say "done: now 'make libsys freebsd-libs freebsd-progs freebsd-terminfo' in $here"
