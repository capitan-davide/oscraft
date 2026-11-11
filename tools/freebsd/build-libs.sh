#!/bin/sh
# Builds FreeBSD's libraries for OsCraft inside FreeBSD's build environment
# (tools/build/make.py ... buildenv BUILDENV_SHELL="sh this-script"): each
# directory given, obj, all and install into the temporary tree (the
# sysroot), with OsCraft's libsys object in place of the syscall stubs.
#   OSCRAFT_LIBSYS_OBJ=<object> build-libs.sh <dir>...
set -e
: "${OSCRAFT_LIBSYS_OBJ:?the libsys object (make libsys)}"
# OSCRAFT_BUILD_FLAGS adds to every make: NO_SHARED=yes for programs
# linked statically.
F="-DWITHOUT_PROFILE -DWITHOUT_TESTS -DWITHOUT_MAN -DWITHOUT_DEBUG_FILES MK_TESTS=no DESTDIR=$SYSROOT -DNO_ROOT METALOG=$SYSROOT/METALOG OSCRAFT_LIBSYS_OBJ=$OSCRAFT_LIBSYS_OBJ $OSCRAFT_BUILD_FLAGS"
mkdir -p "$SYSROOT/libexec" "$SYSROOT/oscraft/bin"
for d in "$@"; do
    echo "=== $d"
    (cd "$d" && make $F obj && make -j16 $F all && make $F install)
done
echo "=== done"
