# Preface

This book describes OsCraft as it is: a real-time microkernel operating
system for 64-bit Arm, written in Rust, whose programs are FreeBSD's.

It is a book about how the system works and why it is built that way.
Each chapter introduces its subject from the ground up, with the ideas
that any system of this kind has to deal with, and then explains
OsCraft's implementation. Figures carry much of the explanation. Code is
shown where the code itself is the clearest statement of a rule, and
every such listing is taken from the source tree when the book is built,
so it is the code the system runs.

The book follows the system's structure. Part I gives the overall
picture and the practical basics. Part II goes through the microkernel,
from the first instruction after boot to processes. Part III covers
process 1, the first process, which manages processes, memory and path
names. Part IV describes the native system built on them: FreeBSD's
binary interface, the library that carries FreeBSD's system calls, and
the servers. Part V explains how FreeBSD's own kernel subsystems, such
as its network stack, run inside servers. Part VI covers the optional
compatibility with QNX programs. Part VII shows how to observe a
running system with traces, debuggers and crash dumps.

The source of the book is in `docs/book/` in the repository. Its README
explains how to build it, and how the listings are tied to the code.
