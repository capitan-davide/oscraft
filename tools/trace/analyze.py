#!/usr/bin/env python3
"""Analyze a kernel trace decoded by QNX's traceprinter.

Usage: analyze.py TRACE.txt [--compare OTHER.txt] [--pid PID] [--top N]

TRACE.txt is traceprinter's text (collect.py's trace.txt, or
collect.py's). Prints:
- overview: span, events per class and per CPU, processes and threads;
- CPU time per thread (from THRUNNING to the thread's next state on
  that CPU, or the CPU's next THRUNNING);
- kernel calls: count, failures (ret_val -1), time from call to return
  on the calling thread (blocking calls include the time blocked);
- scheduling latency: THREADY to THRUNNING per thread, worst cases;
- interrupts per id and message passing per process.

--compare OTHER.txt puts a second trace's numbers beside the first
(rates per second, so traces of different length compare): e.g. the same
workload traced on OsCraft and on the QNX reference image.
--pid limits the per-thread sections to one process.
"""

import argparse
import re
import statistics
from collections import Counter, defaultdict

EVENT = re.compile(r"^t:0x([0-9a-fA-F]+) CPU:(\d+) (\S+)\s*:(\S+)\s*(.*)$")
FIELD = re.compile(r"(\w+(?:\(\w+\))?):(\S+)")


class Trace:
    def __init__(self, path):
        self.path = path
        self.header = {}
        self.events = []  # (time in cycles, cpu, class, name, {fields})
        self.proc = {}  # pid -> name
        self.thread = {}  # (pid, tid) -> name
        msb, last = {}, {}
        for line in open(path, errors="replace"):
            line = line.rstrip("\n")
            m = EVENT.match(line)
            if m:
                lsb, cpu, cls, name, rest = int(m[1], 16), int(m[2]), m[3], m[4], m[5]
                fields = dict(FIELD.findall(rest))
                if cls == "CONTROL" and name == "TIME":
                    msb[cpu] = int(fields.get("msb", "0"), 16)
                elif cpu in last and lsb + (1 << 31) < last[cpu]:
                    msb[cpu] = msb.get(cpu, 0) + 1  # the low word wrapped
                last[cpu] = lsb
                self.events.append([(msb.get(cpu, 0) << 32) | lsb, cpu, cls, name, fields])
            elif self.events and ":" in line and line.startswith(" "):
                k, _, v = line.strip().partition(":")
                self.events[-1][4][k] = v
            elif "::" in line:
                k, _, v = line.partition("::")
                self.header[k.strip()] = v.strip()
        self.events.sort(key=lambda e: e[0])
        self.hz = float(self.header.get("TRACE_CYCLES_PER_SEC", "1") or 1)
        for _, _, cls, name, f in self.events:
            if name == "PROCCREATE_NAME" and "pid" in f:
                self.proc[int(f["pid"])] = f.get("name", "?")
            elif name == "PROCTHREAD_NAME" and "pid" in f:
                self.thread[(int(f["pid"]), int(f["tid"]))] = f.get("name", "")
        times = [e[0] for e in self.events]
        self.span = (times[-1] - times[0]) / self.hz if times else 0.0

    def us(self, cycles):
        return cycles * 1e6 / self.hz

    def who(self, pid, tid=None):
        p = f"{pid} {self.proc.get(pid, '?').rsplit('/', 1)[-1]}"
        if tid is None:
            return p
        t = self.thread.get((pid, tid))
        return f"{p} t{tid}" + (f" ({t})" if t else "")


def thread_of(f):
    try:
        return int(f["pid"]), int(f["tid"])
    except (KeyError, ValueError):
        return None


def analyze(t, pid=None):
    """All the numbers, as dicts, for printing and comparing."""
    r = {"classes": Counter(), "cpus": Counter(), "cputime": Counter(), "calls": defaultdict(list),
         "callname": {}, "fails": Counter(), "latency": defaultdict(list), "ints": Counter(),
         "comm": Counter()}
    running = {}  # cpu -> (thread, since)
    pending = {}  # thread -> (call number, time)
    ready = {}  # thread -> time made ready
    for time, cpu, cls, name, f in t.events:
        r["classes"][cls] += 1
        r["cpus"][cpu] += 1
        th = thread_of(f)
        if cls == "THREAD" and th:
            cur = running.get(cpu)
            if name == "THRUNNING":
                if cur:
                    r["cputime"][cur[0]] += time - cur[1]
                running[cpu] = (th, time)
                if th in ready:
                    r["latency"][th].append(time - ready.pop(th))
            else:
                if cur and cur[0] == th:
                    r["cputime"][th] += time - cur[1]
                    del running[cpu]
                if name == "THREADY":
                    ready.setdefault(th, time)
        elif cls in ("KER_CALL", "KER_EXIT"):
            cur = running.get(cpu)
            num = name.rsplit("/", 1)[-1]
            if cls == "KER_CALL":
                r["callname"].setdefault(num, name.rsplit("/", 1)[0])
                if cur:
                    pending[cur[0]] = (num, time)
            else:
                r["callname"].setdefault(num, name.rsplit("/", 1)[0])
                if f.get("ret_val") in ("-1", "0xffffffff", "0xffffffffffffffff"):
                    r["fails"][num] += 1
                if cur and cur[0] in pending and pending[cur[0]][0] == num:
                    r["calls"][num].append(time - pending.pop(cur[0])[1])
        elif cls == "INTERRUPT":
            r["ints"][(name, f.get("intr_id", "?"))] += 1
        elif cls == "COMM" and "pid" in f:
            r["comm"][(name, int(f["pid"]))] += 1
    # Exits counted per call, matched or not.
    r["exits"] = Counter(e[3].rsplit("/", 1)[-1] for e in t.events if e[2] == "KER_EXIT")
    if pid is not None:
        r["cputime"] = Counter({k: v for k, v in r["cputime"].items() if k[0] == pid})
        r["latency"] = {k: v for k, v in r["latency"].items() if k[0] == pid}
    return r


def pct(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(q * len(xs)))]


def show(t, r, top, other=None, o=None):
    def rate(n, tr):
        return n / tr.span if tr.span else 0.0

    cmp = other is not None
    print(f"== {t.path}: {len(t.events)} events over {t.span * 1e3:.1f} ms, "
          f"{t.header.get('TRACE_CPU_NUM', '?')} CPUs, {t.header.get('TRACE_SYS_VERSION', '?')}")
    if cmp:
        print(f"   compared with {other.path}: {len(other.events)} events over {other.span * 1e3:.1f} ms, "
              f"{other.header.get('TRACE_SYS_VERSION', '?')} (rates per second)")
    print("\n== events per class" + (" (per second: this / other)" if cmp else ""))
    for cls in sorted(set(r["classes"]) | set(o["classes"] if cmp else ())):
        n = r["classes"][cls]
        if cmp:
            print(f"   {cls:<12} {rate(n, t):>10.0f} / {rate(o['classes'][cls], other):<10.0f}")
        else:
            print(f"   {cls:<12} {n:>8}")
    print("   per CPU: " + ", ".join(f"CPU{c} {n}" for c, n in sorted(r["cpus"].items())))

    print(f"\n== CPU time per thread (top {top})")
    total = t.span * len(r["cpus"] or [0]) or 1
    for th, cyc in r["cputime"].most_common(top):
        print(f"   {t.us(cyc) / 1e3:9.2f} ms  {100 * t.us(cyc) / 1e6 / total:5.1f}%  {t.who(*th)}")

    print("\n== kernel calls (call to return, us; blocking calls include the wait)")
    hdr = f"   {'call':<24} {'count':>7} {'fail':>5} {'mean':>8} {'p99':>8} {'max':>8}"
    print(hdr + ("   other: count/s  mean" if cmp else ""))
    for num, n in r["exits"].most_common():
        d = [t.us(x) for x in r["calls"].get(num, [])]
        name = f"{r['callname'].get(num, '?')}/{num}"
        line = f"   {name:<24} {n:>7} {r['fails'][num]:>5}"
        line += f" {statistics.mean(d):>8.1f} {pct(d, .99):>8.1f} {max(d):>8.1f}" if d else " " * 27
        if cmp:
            od = [other.us(x) for x in o["calls"].get(num, [])]
            line += f"   {rate(o['exits'][num], other):>7.0f}/s {statistics.mean(od):>7.1f}" if od else \
                f"   {rate(o['exits'][num], other):>7.0f}/s"
            line += f"   (this {rate(n, t):.0f}/s)"
        print(line)
    if cmp:
        missing = [n for n in o["exits"] if n not in r["exits"]]
        if missing:
            print("   only in the other: " + ", ".join(f"{o['callname'].get(n, '?')}/{n}" for n in missing))

    lat = [x for xs in r["latency"].values() for x in xs]
    print("\n== scheduling latency (THREADY to THRUNNING, us)")
    if lat:
        us = [t.us(x) for x in lat]
        line = f"   {len(us)} wakeups: mean {statistics.mean(us):.1f}, p50 {pct(us, .5):.1f}, " \
               f"p99 {pct(us, .99):.1f}, max {max(us):.1f}"
        if cmp and any(o["latency"].values()):
            ous = [other.us(x) for xs in o["latency"].values() for x in xs]
            line += f"   | other: mean {statistics.mean(ous):.1f}, p99 {pct(ous, .99):.1f}, max {max(ous):.1f}"
        print(line)
        worst = sorted(((max(xs), th) for th, xs in r["latency"].items()), reverse=True)[:top]
        for cyc, th in worst:
            print(f"   worst {t.us(cyc):9.1f} us  {t.who(*th)}")
    else:
        print("   none")

    if r["ints"]:
        print("\n== interrupts")
        for (name, iid), n in r["ints"].most_common(top):
            print(f"   {name:<16} {iid:<14} {n:>7}")
    if r["comm"]:
        print("\n== message passing per process")
        for (name, p), n in r["comm"].most_common(top):
            print(f"   {name:<14} {t.who(p):<32} {n:>7}")


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("trace")
    p.add_argument("--compare", metavar="OTHER")
    p.add_argument("--pid", type=int)
    p.add_argument("--top", type=int, default=15)
    a = p.parse_args()
    t = Trace(a.trace)
    other = Trace(a.compare) if a.compare else None
    show(t, analyze(t, a.pid), a.top, other, analyze(other, a.pid) if other else None)


if __name__ == "__main__":
    main()
