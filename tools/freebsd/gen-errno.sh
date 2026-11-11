#!/bin/sh
# Prints user/proto/src/errno.rs's constants from FreeBSD's
# <sys/errno.h>: rerun when the FreeBSD base moves, and compare.
# Usage: tools/freebsd/gen-errno.sh [freebsd-src]
src=${1:-../freebsd-src}
sed -nE 's/^#define[ \t]+(E[A-Z0-9]+)[ \t]+([0-9]+).*/pub const \1: i32 = \2;/p' "$src/sys/sys/errno.h" | awk '!s[$3]++'
sed -nE 's/^#define[ \t]+(E[A-Z0-9]+)[ \t]+(E[A-Z0-9]+).*/pub const \1: i32 = \2;/p' "$src/sys/sys/errno.h" | awk '!s[$3]++'
