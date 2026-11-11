# A breakpoint hit in a second thread; threads and a backtrace.
file build/qnx-tests/dbgtest
set nto-executable /proc/boot/dbgtest
break add
run
info threads
bt 2
delete
continue
