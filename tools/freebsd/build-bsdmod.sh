#!/bin/sh
# Builds a module of a bsdkern server (netd, pcid): FreeBSD kernel C
# compiled as the server's library is (the flags and renames that
# build-bsdlib.sh saved for it), position-independent, linked with
# user/bsdkern/mods/module.c into a shared object the server dlopen()s.
#   build-bsdmod.sh <library: bsdnet, bsdpci> <out.ko> <source.c>...
set -eu
lib=$1; shift
out=$1; shift
here=$(cd "$(dirname "$0")/../.." && pwd)
bsd=$here/build/freebsd
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
objs=
for c in "$@" "$here/user/bsdkern/mods/module.c"; do
    o=$work/$(basename "$c" .c).o
    # From the library's build directory: its flags name headers relative
    # to it. (Less the device tree's version, whose quotes a shell word
    # loses.)
    (cd "$bsd/$lib-compile" && ${CC:-cc} -c -I"$here/user/bsdkern/mods/include" -I"$here/user/bsdkern/kern" $(sed 's/-DLINUX_DTS_VERSION=[^ ]*//' "$bsd/$lib.cflags") ${MODCFLAGS:-} -fPIC -fvisibility=default "$(realpath "$c")" -o "$o")
    ${OBJCOPY:-objcopy} --redefine-syms="$bsd/$lib.renames" "$o"
    objs="$objs $o"
done
mkdir -p "$(dirname "$out")"
# -Bsymbolic: a module's own symbols bind inside it (its per-CPU set's
# bounds, bsdkern_module_dpcpu_delta), whatever other module is loaded.
${CC:-cc} -shared -nostdlib -fuse-ld=lld -Wl,-z,nostart-stop-gc -Wl,-Bsymbolic -o "$out" $objs
echo "$out"
