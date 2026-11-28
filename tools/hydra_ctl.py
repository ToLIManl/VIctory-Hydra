#!/usr/bin/env python3
"""Hydra Control CLI — unified live control & benchmark client for VIctory-Hydra + RAGE 9.

Connects to the local Windows Named Pipe `\\\\.\\pipe\\victory_hydra` exposed by
`victory.exe` (current-user DACL, PIPE_REJECT_REMOTE_CLIENTS).

Examples:
    python tools/hydra_ctl.py status
    python tools/hydra_ctl.py engine
    python tools/hydra_ctl.py prof --limit 15
    python tools/hydra_ctl.py set VICTORY_PROF 1
    python tools/hydra_ctl.py set VICTORY_AMBIENT 0.8
    python tools/hydra_ctl.py player
    python tools/hydra_ctl.py clock 12 0 0 --pause
    python tools/hydra_ctl.py weather EXTRASUNNY
    python tools/hydra_ctl.py tp -100.0 200.0 30.0
    python tools/hydra_ctl.py god on
    python tools/hydra_ctl.py freeze on
    python tools/hydra_ctl.py timescale 0.5
    python tools/hydra_ctl.py entities
    python tools/hydra_ctl.py native SET_CLOCK_TIME i:12 i:0 i:0
    python tools/hydra_ctl.py bench-lock
    python tools/hydra_ctl.py repl
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sys
import time
from typing import Any

DEFAULT_PIPE = os.environ.get("VICTORY_HYDRA_PIPE", r"\\.\pipe\victory_hydra")


def send_cmd(payload: dict[str, Any], pipe_name: str = DEFAULT_PIPE) -> dict[str, Any]:
    raw = (json.dumps(payload, separators=(",", ":")) + "\n").encode("utf-8")
    with open(pipe_name, "r+b", buffering=0) as fp:
        fp.write(raw)
        buf = bytearray()
        while True:
            ch = fp.read(1)
            if not ch or ch == b"\n":
                break
            buf.extend(ch)
    text = buf.decode("utf-8", errors="replace").strip()
    if not text:
        return {"ok": False, "err": "empty response"}
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {"ok": False, "err": "invalid json", "raw": text}


def pretty(data: dict[str, Any]) -> None:
    print(json.dumps(data, indent=2, ensure_ascii=False))


def parse_typed_arg(spec: str) -> dict[str, Any]:
    """Parses `i:12`, `u:0x1234`, `f:1.5`, `b:true`, `s:EXTRASUNNY`."""
    if ":" not in spec:
        # Auto-infer int / float / bool / string
        low = spec.lower()
        if low in ("true", "false"):
            return {"t": "b", "v": low == "true"}
        try:
            return {"t": "i", "v": int(spec, 0)}
        except ValueError:
            pass
        try:
            return {"t": "f", "v": float(spec)}
        except ValueError:
            pass
        return {"t": "s", "v": spec}
    prefix, val = spec.split(":", 1)
    p = prefix.lower()
    if p in ("i", "int"):
        return {"t": "i", "v": int(val, 0)}
    if p in ("u", "u64", "hex"):
        return {"t": "u", "v": int(val, 0)}
    if p in ("f", "float"):
        return {"t": "f", "v": float(val)}
    if p in ("b", "bool"):
        return {"t": "b", "v": val.lower() in ("1", "true", "on", "yes")}
    if p in ("s", "str"):
        return {"t": "s", "v": val}
    raise ValueError(f"Unknown arg type prefix {prefix!r} in {spec!r}")


def run_once(args: argparse.Namespace) -> int:
    pipe = args.pipe
    cmd = args.subcmd
    try:
        if cmd == "status":
            pretty({"ping": send_cmd({"cmd": "ping"}, pipe), "stats": send_cmd({"cmd": "stats"}, pipe)})
        elif cmd == "engine":
            pretty(send_cmd({"cmd": "engine.status"}, pipe))
        elif cmd == "prof":
            pretty(send_cmd({"cmd": "prof.top", "limit": args.limit}, pipe))
        elif cmd == "vars":
            pretty(send_cmd({"cmd": "settings.list"}, pipe))
        elif cmd == "set":
            pretty(send_cmd({"cmd": "settings.set", "var": args.var, "value": args.value}, pipe))
        elif cmd == "player":
            pretty(send_cmd({"cmd": "game.player"}, pipe))
        elif cmd == "entities":
            pretty(send_cmd({"cmd": "game.entities"}, pipe))
        elif cmd == "clock":
            pretty(send_cmd({
                "cmd": "game.clock",
                "h": args.hour,
                "m": args.minute,
                "s": args.second,
                "pause": 1 if args.pause else 0,
            }, pipe))
        elif cmd == "weather":
            pretty(send_cmd({"cmd": "game.weather", "name": args.name}, pipe))
        elif cmd == "tp":
            pretty(send_cmd({"cmd": "game.teleport", "x": args.x, "y": args.y, "z": args.z}, pipe))
        elif cmd == "god":
            pretty(send_cmd({"cmd": "game.god", "on": 1 if args.state in ("1", "on", "true") else 0}, pipe))
        elif cmd == "freeze":
            pretty(send_cmd({"cmd": "game.freeze", "on": 1 if args.state in ("1", "on", "true") else 0}, pipe))
        elif cmd == "timescale":
            pretty(send_cmd({"cmd": "game.timescale", "scale": args.scale}, pipe))
        elif cmd == "bench-lock":
            pretty(send_cmd({"cmd": "game.bench_lock"}, pipe))
        elif cmd == "native":
            typed = [parse_typed_arg(a) for a in args.args]
            pretty(send_cmd({"cmd": "game.native", "name": args.name, "args": typed}, pipe))
        elif cmd == "watch":
            out_fp = open(args.csv, "w", newline="", encoding="utf-8") if args.csv else None
            writer = None
            end_t = time.time() + args.secs if args.secs > 0 else float("inf")
            try:
                while time.time() < end_t:
                    st = send_cmd({"cmd": "stats"}, pipe)
                    st["ts"] = round(time.time(), 2)
                    if out_fp:
                        if writer is None:
                            writer = csv.DictWriter(out_fp, fieldnames=list(st.keys()))
                            writer.writeheader()
                        writer.writerow(st)
                        out_fp.flush()
                    print(json.dumps(st))
                    time.sleep(args.interval)
            finally:
                if out_fp:
                    out_fp.close()
        elif cmd == "repl":
            print(f"Hydra Control REPL connected to {pipe}. Type JSON or `status|engine|prof|player|entities|bench-lock|quit`.")
            while True:
                try:
                    raw = input("hydra> ").strip()
                except EOFError:
                    break
                if not raw or raw in ("quit", "exit", "q"):
                    break
                if raw.startswith("{"):
                    pretty(send_cmd(json.loads(raw), pipe))
                else:
                    parts = raw.split()
                    sub = build_parser().parse_args(parts)
                    sub.pipe = pipe
                    run_once(sub)
        else:
            print(f"Unknown command: {cmd}", file=sys.stderr)
            return 2
        return 0
    except FileNotFoundError:
        print(f"ERROR: Hydra pipe not found ({pipe}). Is victory.exe running?", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"ERROR: pipe I/O failed ({exc})", file=sys.stderr)
        return 1


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Hydra Control CLI for VIctory-Hydra + RAGE 9")
    p.add_argument("--pipe", default=DEFAULT_PIPE, help="Named pipe path")
    sub = p.add_subparsers(dest="subcmd", required=True)

    sub.add_parser("status", help="Show host ping + live counters")
    sub.add_parser("engine", help="Show RAGE 9 patch status + live verified globals")

    pr = sub.add_parser("prof", help="Show Top-N sampled CPU shaders (requires VICTORY_PROF=1)")
    pr.add_argument("--limit", type=int, default=10)

    sub.add_parser("vars", help="List live vs restart-only settings")

    s = sub.add_parser("set", help="Live-apply a VICTORY_* setting without restart")
    s.add_argument("var")
    s.add_argument("value")

    sub.add_parser("player", help="Read player ped coords, heading, health (requires hydra_bridge.asi)")
    sub.add_parser("entities", help="Count live peds/vehicles/objects (requires hydra_bridge.asi)")

    cl = sub.add_parser("clock", help="Set in-game clock")
    cl.add_argument("hour", type=int)
    cl.add_argument("minute", type=int, nargs="?", default=0)
    cl.add_argument("second", type=int, nargs="?", default=0)
    cl.add_argument("--pause", action="store_true")

    w = sub.add_parser("weather", help="Set persistent weather (e.g. EXTRASUNNY, CLEAR, RAIN)")
    w.add_argument("name")

    tp = sub.add_parser("tp", help="Teleport player ped to X Y Z")
    tp.add_argument("x", type=float)
    tp.add_argument("y", type=float)
    tp.add_argument("z", type=float)

    g = sub.add_parser("god", help="Toggle player invincibility (on/off)")
    g.add_argument("state")

    fr = sub.add_parser("freeze", help="Toggle player position freeze (on/off)")
    fr.add_argument("state")

    ts = sub.add_parser("timescale", help="Set RAGE 9 time scale (e.g. 0.5, 1.0)")
    ts.add_argument("scale", type=float)

    sub.add_parser("bench-lock", help="Freeze clock (12:00), weather (EXTRASUNNY), and player position for A/B benchmarks")

    nv = sub.add_parser("native", help="Call any of 6,829 GTA 6 natives by name or 0xhash")
    nv.add_argument("name", help="Native name (e.g. SET_CLOCK_TIME) or 0x... hash")
    nv.add_argument("args", nargs="*", help="Typed args: i:12 u:0x1 f:1.5 b:true s:text")

    wt = sub.add_parser("watch", help="Stream host stats to stdout and optional CSV")
    wt.add_argument("--secs", type=float, default=30.0)
    wt.add_argument("--interval", type=float, default=1.0)
    wt.add_argument("--csv", default="")

    sub.add_parser("repl", help="Interactive control shell")
    return p


if __name__ == "__main__":
    raise SystemExit(run_once(build_parser().parse_args()))
