# AGENTS.md

Guidance for AI coding agents (and humans) working on OsCraft. Read this
first, then `docs/README.md`. What to work on next is on the project
board: the tasks are GitHub issues, and `CONTRIBUTING.md` says how work
is claimed and merged when several agents work at once: read it before
taking a task.

## What OsCraft is

A microkernel RTOS in Rust for AArch64, running on QEMU's `virt` machine,
with FreeBSD's userland as its own:

- **The system is FreeBSD-native.** The kernel, process 1's managers
  (process, path and memory), the native servers and the clients all use
  FreeBSD's numbers and structs (errnos, signals, `stat`, `termios`,
  `dirent`, `siginfo`, ...), over OsCraft's real-time core of channels,
  messages and pulses. The userland is FreeBSD's, built from source (the
  fork `capitan-davide/freebsd-src`, branch `oscraft`: stable/15 plus the
  commits kept as patches in `tools/freebsd/patches/`) for the unchanged
  triple `aarch64-unknown-freebsd` with upstream clang and Rust: libc,
  libthr, librt, rtld, sh and the utilities, at `/bin`, `/usr/bin`,
  `/lib` and `/usr/lib`. Below libc sits **libsys** (`user/libsys`,
  Rust), FreeBSD's system-call layer re-done as OsCraft kernel calls and
  resource-manager messages. A program for OsCraft is a FreeBSD program;
  the SDK (`make sdk`) is that sysroot with the toolchain files. FreeBSD's
  network stack, USB stack and PCI bus run in servers (`netd`, `usbd`,
  `pcid`) over `bsdkern`, a kernel environment inside a process.
- **QNX 8.0 compatibility is optional, and second-class.** The kernel's
  call and message ABI is binary-compatible with QNX 8.0's `procnto`, so
  a QNX program (one whose ELF `EI_OSABI` is not FreeBSD's) runs
  unmodified with the QNX personality: it finds QNX's libc and loader and
  a translation service, `qnxd`, under `/compat/qnx`, which a QNX process
  looks in first for any path, as FreeBSD's Linux emulation does in
  `/compat/linux`. The compatibility code stays in the tree
  (`kernel/src/personality/qnx.rs`, `user/qnxd`, `user/qnxabi`), but
  QNX's own programs and libraries come only from the user's licensed
  QNX SDP, at image-build time, with `QNX=1`. Without the SDP the system
  builds, runs and is tested with no QNX code at all.

## Development goals

1. **The system speaks FreeBSD.** Everything native uses FreeBSD's
   numbers and structs; a native process gets them directly, with no
   translation. Names follow that: the plain name is the native thing
   (`Stat`, `Termios`, `errno`, `Personality::Native`), and QNX's variant
   carries the prefix (`QnxStat`, `QnxTermios`, `qnx_errno`,
   `Personality::Qnx`). Only QNX processes are converted, at the boundary
   (`kernel/src/personality/qnx.rs`, `user/qnxabi`, `qnxd`).
2. **QNX is opt-in, and procnto is its measure.** What a QNX process sees
   must match procnto: the QNX 8.0 documentation states the intent, and
   where it is silent procnto's observed behaviour is the specification:
   return values, errno, signal order, timings at tick granularity and
   `/proc` contents. One deliberate difference: process 1 is named
   `kernel`, like the kernel's file, not `procnto`. The QNX programs in
   `tests/qnx/` are the oracle; they are not ported or modified.
3. **Determinism before speed.** QNX 8.0 trades performance for
   determinism (for example, superlocked memory: mappings are fully backed
   at map time, with no copy-on-write and no lazy faults). Follow those
   semantics by default. A faster behaviour may exist only as an opt-in.
   Performance work that keeps the semantics is welcome.
4. **QNX 8.0 only**, when QNX is the reference. QNX 7.1 material may be
   used to cross-check, never as the source of a QNX 8 fact. openQNX 6.4
   sources (2009) explain internals but never an ABI.
5. **The kernel stays board-agnostic.** Board specifics live in the
   startup, the system page and callouts, or in user-space servers that
   take their addresses from them. The kernel never uses compile-time
   board constants. Architecture code lives in `kernel/src/arch/aarch64/`.
   QEMU's `virt` is the only target of this tree; a physical board is a
   BSP of its own repository, built on `startup/`.
6. **Keep the real-time core recognisable.** The scheduler and the IPC
   core stay close to procnto's structure, so that its symbols still help
   when reverse-engineering; what differs by ABI lives in
   `kernel/src/personality/`.
7. **Additions are Rust**, preferably: libsys, the tools, the startup,
   the native servers. FreeBSD's kernel code is ported as it is, over
   `bsdkern`.

## Clean room and licensing

- QNX binaries are read for interface facts only: ABI, message formats and
  observable behaviour. Never copy QNX code. Describe what a procnto
  function does in your own words, with a citation (function name), in
  docs and comments.
- The licensed QNX SDP 8.0 is the user's, installed wherever they chose,
  and optional. Source its `qnxsdp-env.sh` (bash) before building with
  `QNX=1` or using its tools: the build and the tools reach it only
  through the variables it sets, `QNX_HOST` and `QNX_TARGET`, and stop
  when they are unset. **Never write the SDP's install path** in docs,
  build files, scripts or comments; write `$QNX_HOST` or `$QNX_TARGET`
  instead.
- **Never patch or bypass licence checks** in QNX tools.
- Reference material (the QNX docs index, SDP 8.0.5 headers and binaries,
  openQNX 6.4) is in `qnx-documentation/`, which is not part of this
  repository. See the table in `docs/README.md`.

## Repository layout

| Path | Contents |
|---|---|
| `kernel/src/` | the kernel: `arch/aarch64`, scheduler (`sched`, `sporadic`), `clock`, `timeout`, `smp`, `ipi`, `percpu`, `syspage` |
| `kernel/src/personality/` | what differs by a process's personality: `native.rs` (FreeBSD's ABI, the system's) and `qnx.rs` (the conversions for a QNX process) |
| `kernel/src/ker/` | kernel calls: channels, messages, sync, signals, timers, threads, interrupts, kpipe, ... |
| `kernel/src/proc/` | process 1's managers: spawn, procfs, imagefs, shmem, sessions, tty, ... |
| `kernel/src/vm/` | physical memory, address spaces, mappings, kernel heap |
| `startup/` | the startup library (Rust) and its board crate for QEMU's virt, `boards/qemu-virt`: builds the system page and starts the kernel; its own book in `startup/docs/book` |
| `user/libsys/` | libsys: FreeBSD's system calls as OsCraft kernel calls and resource-manager messages (Rust, one object linked into FreeBSD's libc, libsys.so and rtld) |
| `user/proto/` | the layouts of the kernel-call and message interfaces: the system's (`errno`, `fcntl`, `stat::Stat`, `termios::Termios`) and QNX's (`qnx_errno`, `qnx_fcntl`, `QnxStat`, `QnxTermios`) |
| `user/qnxabi/` | the conversions between QNX's and FreeBSD's ABIs, pure functions, compiled into libsys, qnxd and the kernel |
| `user/resmgr/`, `user/kercall/` | the resource-manager framework and the kernel calls, for native servers |
| `user/pipe/`, `user/random/`, `user/pty/`, `user/tmpfs/` | native servers (`make native`): pipes over the kernel's, the random device, pseudo-terminals, tmpfs |
| `user/logring/`, `user/syslogd/` | per-process log rings, and FreeBSD's `syslogd` in Rust |
| `user/bufcache/`, `user/msdosfs/`, `user/md/`, `user/diskd/`, `user/fsd/` | block cache, FAT, memory and image disks, and the servers joining them (`diskd-md`) |
| `user/bsdkern/`, `user/netd/`, `user/usbd/`, `user/pcid/` | FreeBSD's kernel code in a process (`bsdkern`), and the servers built on it: the network stack, the USB stack, the PCI bus; their modules (`kldload`) |
| `user/qnxd/` | the QNX personality's translation service, under `/compat/qnx` |
| `tests/native/hello/`, `tests/qnx/qnxhello/`, `tests/native/priochain/`, `tests/native/bsdkern/` | the tests' Rust programs: a minimal program without a libc per personality, a native server for `priochaintest`, bsdkern's checks |
| `tests/target.tests` | the target test suite (format in `tools/test/run-tests.py`): the native tests, and the QNX tests marked `needs: qnx` |
| `tests/native/` | C test programs for the native personality, built against the sysroot (`make native-tests`) |
| `tests/qnx/` | C test programs for the QNX personality, built with the SDP's GCC against QNX's libc: the oracle (`make qnx-tests`, QNX=1) |
| `tools/freebsd/` | FreeBSD's bootstrap (`bootstrap.sh`), the patches that make the fork, the library and module build scripts |
| `tools/sdk/` | the SDK's toolchain files and its out-of-tree check |
| `images/` | mkifs build files: the system for QEMU's virt, the image `make run` boots (`qemu-virt.build`), and the optional QNX overlay (`qnx.build`). What only the tests add is theirs: `tests/native/image.build`, `tests/qnx/image.build`, and the QNX reference image (`tests/qnx/reference.build`) |
| `tools/mkifs/` | clean-room mkifs and dumpifs |
| `tools/qemu/run.sh` | running an image in QEMU's virt |
| `tools/console.py` | a console on QEMU's socket, from Python |
| `tools/qnx/probe.py` | runs one C program on QNX's procnto and on OsCraft, for comparison (SDP) |
| `tools/trace/`, `tools/kdump/` | kernel traces (QNX's tracelogger) and crash dumps (QNX's kdumper): both need the SDP |
| `tools/doctor.sh` | `make doctor`: what the host is missing |
| `docs/book/` | the book: the system explained, idea first and then the implementation, chapter by chapter, and the design record |
| `docs/notes/` | local working notes, a scratch area that is never committed (`.gitignore`): what is to last goes in the book, an issue or a PR |
| `docs/setup.md` | setting up a development host |

## Build and test

Rust is pinned by `rust-toolchain.toml`; the kernel target is
`aarch64-unknown-none-softfloat` (no FP/SIMD in kernel code). You also need
clang, ld.lld and `qemu-system-aarch64`, FreeBSD's source set up once
(`make freebsd-bootstrap`) and a Rust nightly for libsys: `docs/setup.md`
has the whole recipe, and `make doctor` checks the host. The QNX SDP is
needed only for `QNX=1` and the `qnx-*` targets.

```sh
make run                      # build and boot OsCraft in QEMU (Ctrl-A X quits)
make debug                    # the same, waiting for gdb on :1234
make test                     # the native suite in QEMU (debug kernel)
make test PROFILE=release     # the same with the release kernel
make test TESTS="netd files"  # only the named tests
make test STOP=1              # stop at the first failure
make run SELFTEST=1           # also the kernel's own boot self-tests (off by default)
make clippy                   # the kernel's lints, for its own target
make -C startup test check    # the startup library's unit tests and lints
make test QNX=1               # with the QNX overlay: the QNX tests too (SDP)
make qnx-test                 # the QNX tests on QNX's own kernel, in QEMU (SDP)
make qnx-run                  # boot QNX's kernel in QEMU (SDP)
make doctor                   # what this host is missing
make freebsd-bootstrap        # FreeBSD's source and build system, once
make libsys freebsd-libs freebsd-progs freebsd-terminfo   # the FreeBSD userland, in that order
make sdk sdk-check            # the SDK tarball, and a C and a Rust program built against it, run in QEMU
```

- The native suite (debug and release kernels) must pass before a change
  is committed. A change to the QNX personality also passes `make test
  QNX=1` and, where it changes what a QNX program sees, `make qnx-test`
  on the same tests. A change to the tools' Rust code also passes `cargo
  test -p mkifs` and adds no `cargo clippy -p mkifs` warning.
- **Run one QEMU at a time.** Concurrent runs (a background `make test`
  plus a probe) fail in bulk and look like kernel regressions.
- The `shell` test (`echo a b | cat`) has a rare known flake: a `dup2()`
  race against `SIGCHLD`. Rerun it before suspecting a change.
- Some hangs reproduce only in a particular full test sequence, because the
  harness starts while boot services are still starting. Bisect by code
  (`git stash push -- kernel`, then swap single files back to `HEAD`), not
  by adding prints or gdb, which change the timing.

### Measuring procnto

For a QNX-personality behaviour that the docs don't settle:

1. Write a small C program (a scratch file is fine) that prints what it
   observes.
2. Run it on both kernels:
   ```sh
   python3 -I tools/qnx/probe.py --diff prog.c [args...]
   ```
   It builds the program with the SDP's GCC, adds it as
   `/proc/boot/probe` to the QNX reference image and to OsCraft's image
   with the overlay (in `build/probe/`), boots each in QEMU's virt and
   prints both outputs. `--oscraft-only` skips the QNX run while
   iterating, and `--procnto-opts "-C 1"` passes kernel options to both.
3. Make OsCraft's output identical, then turn the program into a test in
   `tests/target.tests` marked `needs: qnx`. Where timing matters, prefer
   absolute deadlines and wide enough windows to be stable.

When behaviour depends on internals, read procnto's code in
`procnto-smp-instr` with a disassembler or decompiler (it has full
symbols), and record the finding on the issue or in the PR, and in the
book where it explains the code.

### Debugging aids

- `kernel-trace` skill: QNX's tracelogger and traceprinter, compared with a
  trace of the same workload on procnto (`tools/trace/`); needs the SDP
  and `QNX=1`.
- `kernel-dump` skill: crash dumps with QNX's kdumper, read with kdserver
  and gdb (`tools/kdump/`); needs the SDP.
- QNX documentation: the `qnx-docs` MCP server (wired per machine in a
  local `.mcp.json`, which is not in the repository: it runs the server of
  the user's `qnx-developer-docs` checkout) and the `qnx-docs-lookup`
  skill. Use `lookup_symbol` for exact names and `search` for concepts,
  and read a function page's Returns, Errors and Caveats.

## Working with other agents

The tasks are GitHub issues on the project board; `CONTRIBUTING.md` has
the rules and the `project-tasks` skill the commands. In short:

- **Claim before you start, if you plan to deliver**: assign yourself
  the issue and say so in a comment; the assignee is all another agent
  sees before starting the same work, so it means a fix is coming, not
  a bookmark. Unassign what you drop. An issue assigned to someone else
  is theirs.
- **Your own branch and tree**: `work/<id>` from `main`, one clone or
  worktree per agent, never a shared working tree. Every fix reaches
  `main` through a pull request linked to its issue (`Closes #N`), never
  a direct push; the PR template asks for the measured results and the
  suites' lines.
- **One QEMU per machine**: take `/tmp/oscraft-qemu.lock` (`mkdir`, which
  is atomic) around every `make test`, probe or run, and give it back.
- **The docs change in the same PR**: a behaviour the book or
  a skill describes is updated where it is described; the PR's
  description records what was measured and what still differs.
- **Talk on the issue or PR**, so that agents on other machines see it.

## Conventions

- **Code:** match the surrounding code's naming, idiom and comment density.
  Doc comments say what the code does and why, and name the FreeBSD
  function or the procnto function or doc page that backs a non-obvious
  rule. Keep `unsafe` in small, documented places: one operation per
  block, each with a `// SAFETY:` comment; `make clippy` checks it
  (`[workspace.lints]` in `Cargo.toml`). A change adds no clippy warning.
- **Tests:** every behaviour fix comes with a check in a test program
  wired into `tests/target.tests`: a `tests/native/*.c` program (the
  `native-tests` target) for the native personality, whose expected values
  are FreeBSD's semantics, or a `tests/qnx/*test.c` program (the
  `qnx-tests` target) for the QNX personality, whose expected values are
  procnto's measured results.
- **Docs:** the record of a step is its PR (what was done, what was
  measured, what still differs, how to reproduce) and the issues it
  closes or opens. Design decisions go in the book, where the code they
  shape is explained.
- **The book** (`docs/book/`) includes code through `// ANCHOR: name` and
  `// ANCHOR_END: name` comments in the source. Keep them: when you move
  code, move its markers with it. When you change code between markers,
  update the book text that explains it (`git grep` the anchor's name in
  `docs/book/src`), and any figure that draws it (`docs/book/src/images/`).
  Run `make -C docs/book check` before committing; it also fails when a
  new file under `kernel/src` has no row in the book's source map. The
  startup library's book has its own check (`make -C startup book-check`).
- **Commits:** small and focused, one finished subtask each, pushed every
  one or two commits to your branch. Prefixes like `kernel:`, `docs:`,
  `tools:`, after the issue ID when there is one. Commit only your own
  paths (`git commit -- <paths>`; `git add` new files first): the user
  may have unrelated work staged or untracked.
- **Don't touch the user's files** without asking, including untracked
  files you didn't create.
- **Reporting:** state the measured result, say what still differs (from
  FreeBSD's semantics, or from procnto for the QNX personality), and say
  if a test was skipped or failed.
