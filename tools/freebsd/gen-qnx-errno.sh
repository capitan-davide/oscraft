#!/bin/sh
# Prints user/proto/src/qnx_errno.rs's constants from QNX's <errno.h>: rerun
# when the SDP moves, and compare. Needs the SDP's environment.
: "${QNX_TARGET:?source qnxsdp-env.sh first}"
sed -nE 's/^#define[ \t]+(E[A-Z0-9]+)[ \t]+([0-9]+).*/pub const \1: i32 = \2;/p' "$QNX_TARGET/usr/include/errno.h"
sed -nE 's/^#define[ \t]+(E[A-Z0-9]+)[ \t]+(E[A-Z0-9]+).*/pub const \1: i32 = \2;/p' "$QNX_TARGET/usr/include/errno.h"
