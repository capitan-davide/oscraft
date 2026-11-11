# OsCraft documentation

## Setting up

- [setup.md](setup.md) - the host, FreeBSD's source and the build order,
  and the optional QNX SDP; `make doctor` checks it.

## The book - the system explained

[book/](book/README.md): OsCraft chapter by chapter, each idea first and
then the implementation: the microkernel, process 1, the FreeBSD-native
system (libsys, the servers, the userland), FreeBSD's kernel code in
servers (bsdkern, the network stack, PCI, USB, disks), the optional QNX
compatibility, and how to observe a running system. Its listings are
included from the sources (`make -C docs/book`); appendix C maps the
source tree to the chapters.

The startup library has a book of its own, `startup/docs/book`.

## Working notes

`docs/notes/` is a local scratch area for whoever works on the tree,
ignored by git. Nothing there is part of the repository: what is to last
goes in the book, an issue or a pull request.

## Tools

- `tools/test/run-tests.py` - the target suite's harness and file format.
- `tools/qnx/probe.py` - one C program on QNX's procnto and on OsCraft,
  for comparison (AGENTS.md, "Measuring procnto"; needs the SDP).
- `tools/trace/`, `tools/kdump/` - kernel traces and crash dumps with
  QNX's tracelogger and kdumper (the SDP), with the `kernel-trace` and
  `kernel-dump` skills in `../.claude/skills/`.
- `tools/sdk/` - the SDK's toolchain files and check (`make sdk`).

## Reference material

Not part of the repository, and needed only for the QNX personality:

| Where | What |
|---|---|
| `qnx-documentation/index.sqlite`, `mirror/` | the QNX 8.0 and 7.1 documentation, indexed (the `qnx-docs` MCP server, `.mcp.json`) |
| `qnx-documentation/srcs/qnx805/` | SDP 8.0.5 for reference (headers, target binaries) |
| `qnx-documentation/srcs/qnx710/`, `srcs/openqnx/` | QNX 7.1 and openQNX 6.4: for cross-checks and internals, never a QNX 8 fact |
| the QNX SDP 8.0 (installed by the user, anywhere) | reached through its environment, `QNX_HOST` and `QNX_TARGET`: source its `qnxsdp-env.sh`. Its licence checks are never bypassed. `$QNX_TARGET/aarch64le/boot/sys/procnto-smp-instr` keeps its symbols (not stripped) |

## Working on it

[../CONTRIBUTING.md](../CONTRIBUTING.md) - tasks are GitHub issues on the
project board; how to claim, branch, test, and get a pull request merged.
