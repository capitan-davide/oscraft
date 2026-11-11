# Contributing to OsCraft

How work on OsCraft is organized, for people and for AI agents, on one
machine or many. `AGENTS.md` says what OsCraft is and how to build and
test it; this file says how to pick work, do it without colliding with
others, and get it merged.

## 1. Where the work is

- **GitHub issues** are the tasks: one per review finding (`CORE-1`,
  `SCHED-4`, ...), one per roadmap item or other job. Labels: `kind:*`
  (bug, quirk, design, docs, improvement, qnx-bug), `area:*` (core, sched,
  ipc, procmgr, pathmem, tools, docs), `severity:*`, `measure-first`
  (probe procnto before fixing), `needs-board`, `review`, `task`.
- **The project board** ("OsCraft", on the repository's Projects tab)
  shows every issue with a Status: Todo, In progress, In review, Done.
  The issue is the source of truth; the board is its view.
- **Reviews** are issues too: a whole-codebase review becomes one issue
  per finding plus a pinned tracking issue with the method, the themes
  and the open questions (for 2026-10-27: #46). The issues are the
  record, and the fix's PR closes them.
- **Milestones** are the phases; the current one holds every open issue,
  review finding or task until its goal is met, and a new issue goes on
  it unless it belongs to a later phase. The board's current milestone
  says what the current phase is and what is next in it.
- **The `Priority` field** of the board (Now, Next, Later) is the order
  within the milestone: Now for what blocks other work, Next for what
  comes after, Later for the rest. The maintainer sets it; sort the board
  by it.
- **The `roadmap` label** marks the remaining differences from procnto
  and the parts not yet implemented, the former `docs/ROADMAP.md`:
  each one cites the probe that measured it and says what "done"
  means. Measure on procnto first, as for any behaviour. A step that
  finds a new difference opens one.

## 1a. Everyone goes through a pull request

Every fix, on every machine, is linked to its issue through a pull
request: a `work/<id>` branch, a PR that says `Closes #N`, a review, a
merge by the maintainer. That holds for the maintainer's own agents as
well as for external contributors; nothing lands on `main` by a direct
push except the maintainer's own housekeeping.

The maintainer's agents (several Claude Code sessions on one machine,
one GitHub identity) additionally coordinate directly: they take log
entry numbers from ranges they agree on, share QEMU through the lock
below, and may review each other's PRs, with the maintainer merging.

## 2. Starting from scratch

1. Clone the repository. Read `AGENTS.md`, then `docs/README.md` and
   the current milestone on the board
   (`gh issue list --milestone <name>`).
   `docs/setup.md` says what the host needs; `make doctor` checks it.
2. Get the toolchain (`rust-toolchain.toml` pins Rust; clang, ld.lld,
   `qemu-system-aarch64`) and FreeBSD's source (`make freebsd-bootstrap`).
   For the QNX personality, the licensed QNX SDP 8.0 and the reference
   material in `qnx-documentation/` (not in the repository: ask the
   maintainer): install the SDP anywhere and source its `qnxsdp-env.sh`
   (`QNX_HOST`, `QNX_TARGET`). Without it everything but the QNX overlay
   and the QNX tests builds and runs.
3. `make test` must pass before you change anything. If it doesn't, that
   is the first issue to open.
4. `gh auth login` with a token that has `repo` (and `project` for the
   board). Agents coordinate through GitHub only: nothing on another
   machine can see your terminal.

## 3. Claiming a task

- Pick an open, unassigned issue. Prefer the top of the board (highest
  severity first within an area), or the issue you were pointed at.
- **Claim = assign yourself** (`gh issue edit N --add-assignee @me`) and
  move the board item to In progress. Say in a comment what you'll do,
  and which machine or session you are, so a second agent on the same
  item sees it.
- **Claim an issue if you plan to deliver its fix**, as a pull request.
  The assignee is what another agent, on another machine, sees before
  starting the same work, so a claim is a plan to deliver, not a
  bookmark: don't claim what you only find interesting or intend to
  look at later. One or two claims at a time; if you stop, unassign
  yourself and say in a comment where you got to.
- An issue assigned to someone else is theirs. Comment if you have
  something for it; don't work on it.
- `measure-first` means: write the probe, run it on both kernels
  (`python3 -I tools/qnx/probe.py --diff prog.c`), put the result in the
  issue, then fix.

## 3a. Issues are reviewed too

The maintainer triages new issues as well as pull requests. An issue
that is wrong (the behaviour is procnto's too, the measurement is
missing, the code it cites doesn't do what it says), that goes against
the development goals of `AGENTS.md` (a non-QNX behaviour as the
default, a board constant in the kernel, a change to QNX's own parts)
or against the project's way of working gets a comment asking for a
change, or is closed as not planned with the reason. Answer on the
issue; a closed issue can be reopened with the missing measurement or
argument.

## 4. Doing the work

- **One branch per issue** (or a small batch of related ones), named
  `work/<id>` such as `work/sched-4`; commits start with the ID. Never
  work on `main` directly, and never share a working tree with another
  agent: on one machine, each agent has its own clone or `git worktree`.
- **One QEMU per machine.** Before any `make test`, `probe.py` or
  `make run`, take the machine's lock and give it back after:
  ```sh
  until mkdir /tmp/oscraft-qemu.lock 2>/dev/null; do sleep 5; done
  make test; rmdir /tmp/oscraft-qemu.lock
  ```
  A full suite is about ten minutes per profile.
- **The documentation changes with the code, in the same PR.** If a
  change alters anything the book (`docs/book/`) or a skill describes
  (a behaviour, a rule, a layout, a number), the PR updates that text
  and any figure that draws it; the PR's description records what was
  done and measured, but it never replaces the current description. `make -C docs/book check` catches a
  moved listing, not a sentence that stopped being true: read the pages
  that cite the code you touched (`git grep` the anchor's name and the
  function's name in `docs/book/src`). A PR that changes documented
  behaviour without the documentation is not complete.
- Everything in `AGENTS.md` "Conventions" applies: match the code's
  style, measure procnto, a test for every behaviour fix, keep the
  book's anchors, both suites green before a commit.

## 5. Getting it merged

- Open a pull request from your branch; the template asks for the
  measured table, both suites' results and the checklist. `Closes #N`
  links the issue; the merge closes it and moves the board item.
- A PR is reviewed by the maintainer or by another agent (anyone may
  review; say so in a comment). Review comments are answered on the PR,
  with a commit or a reason. Small, focused PRs merge fastest.
- The merge is the maintainer's, as a merge commit (`gh pr merge --merge
  --delete-branch`): commits are signed, and a rebase or squash merge
  would strip the signatures. Rebase on `main` if asked; don't force
  push over someone else's review.
- Continuous integration (`.github/workflows/`) checks the book; the
  target suites can't run on GitHub's runners (licensed SDP, QEMU), so
  the PR's suite lines are what the reviewer trusts. Report them exactly,
  with the commit they ran on.
- **A rebase doesn't always need a rerun.** When the rebase onto `main`
  brings in only commits that merged through their own PRs with their
  own suites, replays without a conflict (the log index row apart), and
  your commits touch no file those commits changed, the pre-rebase
  results stand: say so in the PR ("suites ran on `<sha>`, rebased onto
  `<sha>`, disjoint files"). The maintainer checks the file sets at merge
  time. Rerun both suites when the files overlap, when the rebase had a
  conflict, or when `main` changed the kernel in a way your fix depends
  on. After a batch of such merges the maintainer runs both suites on
  `main` once; a failure there blocks further merges until it's found.

## 6. Talking to each other

- Durable, cross-machine: the issue and PR comments. Decisions, probe
  results and "I'm leaving this" go there.
- Design decisions go in the book, where the code they shape is
  explained; surprising facts about procnto in the PR and in the book.
- Agents on one machine may also message each other directly (Claude
  Code sessions see each other), but what matters still goes on GitHub.
- If you find a bug while doing something else, open an issue (the
  Finding template) rather than fixing it in passing; link it from the
  PR you are on.

## 7. Reporting a QNX defect

A finding that turns out to be procnto's too (the probe shows the same
wrong result on both kernels) is kind `qnx-bug`: OsCraft keeps matching
procnto, the test asserts the measured behaviour, the PR lists it under
the differences that remain, and the maintainer reports it to QNX.
Record the probe and both outputs in the issue.
