# Through qconn: a program uploaded from the host, run, a breakpoint hit
# in a second thread, its threads and backtrace, and its exit.
# The first stop after `run` comes through pdebug and the serial relay,
# on a dev kernel at the end of a whole suite: gdb's default 2-second
# wait for it timed out about one run in four (issue #210).
set remotetimeout 20
file build/qnx-tests/dbgtest
upload build/qnx-tests/dbgtest /tmp/dbgtest-up
set nto-executable /tmp/dbgtest-up
break add
run
info threads
bt 2
delete
continue
