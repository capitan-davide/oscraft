# A fault reported as its signal.
file build/qnx-tests/dbgtest
set nto-executable /proc/boot/dbgtest
run crash
bt 1
kill
