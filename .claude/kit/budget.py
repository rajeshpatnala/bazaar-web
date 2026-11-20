#!/usr/bin/env python3
"""Token budget ledger for the agent team (used by the manager).

Agent runs are recorded AUTOMATICALLY: a SubagentStop hook reads each agent's own transcript and writes the
measured numbers here, against the task set with `budget.py task <id>`. A Stop hook measures the manager's own
session the same way. Limits come from the "Budget" table in .agents/team.md.

Two numbers per run:
  tokens     the agent's context at its last step (what Claude Code shows per agent; budgets and caps use this)
  processed  everything sent and received over all its steps, cache reads included
  start      the context of its first step: what it loaded before doing any work (the cold start)
  start_cached  the part of that start read from the prompt cache (shared prefix of an earlier run, ~10% of the price)
  cost_eq    what the run costs in "plain input tokens": fresh input x1, cache writes x1.25, cache reads x0.1,
             output x5 - the number to compare runs by money, not by size

  budget.py task <id>                            set the task that automatic rows are booked against
  budget.py add <task> <agent> <tokens> [--estimate] [note]
                                                 manual row (only for runs the hooks could not measure);
                                                 --estimate marks a guess, shown separately
  budget.py status [task]                        compact status (task + today + manager overhead)
  budget.py report [task]                        per-agent breakdown (tokens, cold start, work) for reports and retros
  budget.py starts                               typical cold-start size per agent type (what to slim)
  budget.py check <task> <agent>:<count>...      can we afford the next launches?
                                                 exit 0 ok, 1 over warn threshold, 2 over limit
  budget.py hook-subagent | hook-session         used by the hooks (JSON on stdin)
"""
import csv
import json
import datetime as dt
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
LEDGER = ROOT / ".agents" / "budget.csv"
TEAM = ROOT / ".agents" / "team.md"
FIELDS = ["date", "time", "task", "agent", "tokens", "note", "source", "processed", "start", "session", "start_cached", "cost_eq"]
WEIGHTS = {"input_tokens": 1.0, "cache_creation_input_tokens": 1.25, "cache_read_input_tokens": 0.1, "output_tokens": 5.0}
CURRENT_TASK = ROOT / ".agents" / "current-task"
SESSIONS = ROOT / ".agents" / "budget-sessions.json"

# Fallback estimates (tokens per launch) until the ledger has history for an agent type.
DEFAULT_ESTIMATES = {
    "tech-lead": 120_000, "architect": 150_000, "cicd": 150_000, "tester": 120_000,
    "code-reviewer": 70_000, "security-reviewer": 70_000, "build-runner": 25_000,
}
DEVELOPER_ESTIMATE = 200_000
OTHER_ESTIMATE = 100_000


def limits() -> dict:
    """Read 'setting | value' rows from the Budget section of team.md."""
    out = {"per_task_tokens": 0, "per_day_tokens": 0, "warn_percent": 80, "usd_per_million_tokens": 0.0}
    text = TEAM.read_text() if TEAM.exists() else ""
    section = re.search(r"^## Budget.*?(?=^## |\Z)", text, re.S | re.M)
    for line in (section.group(0) if section else "").splitlines():
        cells = [c.strip().strip("`") for c in line.strip().strip("|").split("|")]
        if len(cells) >= 2 and cells[0] in out:
            raw = cells[1].replace(",", "").replace("_", "").strip()
            if raw in ("", "-", "none", "unlimited"):
                continue
            try:
                out[cells[0]] = float(raw) if cells[0] == "usd_per_million_tokens" else int(float(raw))
            except ValueError:
                sys.exit(f"budget: bad value for {cells[0]} in team.md: {cells[1]}")
    return out


def measure() -> str:
    """What the caps count (team.md -> budget_measure): `tokens` = each run's final context (the old figure, far below
    the real cost), `cost-eq` = what the runs really cost in plain-input tokens, with the manager included."""
    return "cost-eq" if team_value("budget_measure", "tokens").lower() in ("cost-eq", "cost_eq", "costeq") else "tokens"


def used_by(history: list, keep, today: str = "") -> int:
    """Usage of the selected rows in the configured measure (cost-eq falls back to tokens for unmeasured rows);
    with cost-eq and a day, the manager's own sessions of that day are added."""
    if measure() == "tokens":
        return sum(int(r["tokens"]) for r in history if keep(r))
    total = sum(int(float(r.get("cost_eq") or 0)) or int(r["tokens"]) for r in history if keep(r))
    if today:
        try:
            sessions = json.loads(SESSIONS.read_text()) if SESSIONS.exists() else {}
        except ValueError:
            sessions = {}
        total += sum(int(v.get("cost_eq", 0)) for v in sessions.values() if v.get("last") == today)
    return total


def rows() -> list:
    if not LEDGER.exists():
        return []
    with LEDGER.open() as f:
        out = list(csv.DictReader(f))
    for r in out:
        r["source"] = r.get("source") or "manual"
        r["processed"] = r.get("processed") or ""
        r["start"] = r.get("start") or ""
        r["session"] = r.get("session") or ""
    return out


def write_row(row: dict) -> None:
    LEDGER.parent.mkdir(parents=True, exist_ok=True)
    if LEDGER.exists():
        with LEDGER.open() as f:
            header = next(csv.reader(f), [])
        if header != FIELDS:  # older ledger: rewrite with the new columns
            old = rows()
            with LEDGER.open("w", newline="") as f:
                w = csv.DictWriter(f, fieldnames=FIELDS, extrasaction="ignore")
                w.writeheader()
                w.writerows(old)
    new = not LEDGER.exists()
    with LEDGER.open("a", newline="") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS, extrasaction="ignore")
        if new:
            w.writeheader()
        w.writerow(row)


def transcript_usage(path: str, with_start: bool = False) -> tuple:
    """(context at last step, processed over all steps, steps[, start]) from a Claude Code transcript (JSONL).
    start = the context of the first step: what the agent loaded before doing any work (its cold-start cost)."""
    seen, last, steps, start = {}, 0, 0, 0
    try:
        with open(pathlib.Path(path).expanduser(), errors="replace") as f:
            for line in f:
                if '"usage"' not in line:
                    continue
                try:
                    obj = json.loads(line)
                except ValueError:
                    continue
                msg = obj.get("message") or {}
                u = msg.get("usage") if isinstance(msg, dict) else None
                if obj.get("type") != "assistant" or not u:
                    continue
                total = sum(int(u.get(k) or 0) for k in ("input_tokens", "cache_creation_input_tokens",
                                                            "cache_read_input_tokens", "output_tokens"))
                if not seen:
                    start = total - int(u.get("output_tokens") or 0)
                seen[msg.get("id") or len(seen)] = total
                last = total
    except OSError:
        return (0, 0, 0, 0) if with_start else (0, 0, 0)
    return (last, sum(seen.values()), len(seen), start) if with_start else (last, sum(seen.values()), len(seen))


def transcript_detail(path: str) -> dict:
    """Like transcript_usage, plus the cache split of the first step and a cost-equivalent over all steps."""
    seen, out = {}, {"ctx": 0, "processed": 0, "steps": 0, "start": 0, "start_cached": 0, "cost_eq": 0}
    try:
        with open(pathlib.Path(path).expanduser(), errors="replace") as f:
            for line in f:
                if '"usage"' not in line:
                    continue
                try:
                    obj = json.loads(line)
                except ValueError:
                    continue
                msg = obj.get("message") or {}
                u = msg.get("usage") if isinstance(msg, dict) else None
                if obj.get("type") != "assistant" or not u:
                    continue
                n = {k: int(u.get(k) or 0) for k in WEIGHTS}
                if not seen:
                    out["start"] = n["input_tokens"] + n["cache_creation_input_tokens"] + n["cache_read_input_tokens"]
                    out["start_cached"] = n["cache_read_input_tokens"]
                seen[msg.get("id") or len(seen)] = n  # streamed parts of one message repeat its usage: keep the last
                out["ctx"] = sum(n.values())
    except OSError:
        return out
    out["steps"] = len(seen)
    out["processed"] = sum(sum(n.values()) for n in seen.values())
    out["cost_eq"] = int(sum(sum(WEIGHTS[k] * v for k, v in n.items()) for n in seen.values()))
    return out


def first_prompt(path: str) -> str:
    """The task message an agent was launched with (first user message in its transcript)."""
    try:
        with open(pathlib.Path(path).expanduser(), errors="replace") as f:
            for line in f:
                if '"user"' not in line:
                    continue
                obj = json.loads(line)
                if obj.get("type") != "user":
                    continue
                c = (obj.get("message") or {}).get("content")
                if isinstance(c, list):
                    c = " ".join(x.get("text", "") for x in c if isinstance(x, dict))
                return c or ""
    except (OSError, ValueError):
        pass
    return ""


def prompt_task(path: str) -> str:
    """The task id from the dispatch's own `Task: <id>` line, so parallel tasks book separately."""
    m = re.search(r"(?mi)^\s*\**Task:?\**\s*:?\s*([A-Za-z]+\d+[\w.-]*)", first_prompt(path)[:2000])
    return m.group(1) if m else ""


def team_value(name: str, default: str = "") -> str:
    text = TEAM.read_text() if TEAM.exists() else ""
    m = re.search(rf"^\|\s*`?{re.escape(name)}`?\s*\|\s*([^|]*?)\s*\|", text, re.M)
    return m.group(1) if m and m.group(1) not in ("", "-") else default


USAGE_FILE = ROOT / ".agents" / ".usage-limits.json"


def plan_usage() -> dict:
    """The owner's real plan usage as Claude Code last reported it to the status line (5-hour and 7-day windows)."""
    try:
        return json.loads(USAGE_FILE.read_text())
    except (OSError, ValueError):
        return {}


def plan_usage_age():
    """Minutes since the status line last recorded plan usage, or None when it never did."""
    at = plan_usage().get("at")
    return int((dt.datetime.now().timestamp() - at) // 60) if at else None


def plan_usage_line() -> str:
    u = plan_usage()
    parts = []
    for key, label in (("five_hour", "5h"), ("seven_day", "7d")):
        w = u.get(key) or {}
        if "used_percentage" in w:
            reset = ""
            if w.get("resets_at"):
                reset = " (resets " + dt.datetime.fromtimestamp(int(w["resets_at"])).strftime("%a %H:%M") + ")"
            parts.append(f"{label} {float(w['used_percentage']):.0f}%{reset}")
    if not parts:
        return ""
    age = ""
    if u.get("at"):
        mins = int((dt.datetime.now().timestamp() - u["at"]) // 60)
        age = f", as of {mins} min ago" if mins > 2 else ""
    return "plan usage: " + ", ".join(parts) + age


def first_prompt_chars(path: str) -> int:
    """Size of the task message an agent was launched with (first user message in its transcript)."""
    try:
        with open(pathlib.Path(path).expanduser(), errors="replace") as f:
            for line in f:
                if '"user"' not in line:
                    continue
                obj = json.loads(line)
                if obj.get("type") != "user":
                    continue
                c = (obj.get("message") or {}).get("content")
                if isinstance(c, list):
                    c = " ".join(x.get("text", "") for x in c if isinstance(x, dict))
                return len(c or "")
    except (OSError, ValueError):
        pass
    return 0


def current_task() -> str:
    try:
        return CURRENT_TASK.read_text().strip() or "untracked"
    except OSError:
        return "untracked"


def stack_json() -> dict:
    p = ROOT / ".agents" / "stack.json"
    return json.loads(p.read_text()) if p.exists() else {}


def developers() -> set:
    return {d["name"] for d in stack_json().get("developers", [])}


def fmt(n: float) -> str:
    return f"{n/1e6:.2f}M" if n >= 1e6 else f"{n/1e3:.0f}k"


def usage_line(label: str, used: int, limit: int, price: float) -> str:
    cost = f" ~${used / 1e6 * price:,.2f}" if price else ""
    if not limit:
        return f"{label}: {fmt(used)} (no limit){cost}"
    return f"{label}: {fmt(used)} / {fmt(limit)} ({used * 100 // limit}%){cost}"


def agent_caps() -> dict:
    """max_tokens_per_run per agent from the agent tables in team.md."""
    caps, header = {}, None
    for line in (TEAM.read_text() if TEAM.exists() else "").splitlines():
        s = line.strip()
        if not s.startswith("|"):
            header = None
            continue
        cells = [c.strip().strip("`") for c in s.strip("|").split("|")]
        if header is None:
            header = [c.lower() for c in cells] if cells and cells[0].lower() == "agent" else []
            continue
        if not header or "max_tokens_per_run" not in header or set("".join(cells)) <= set("-: "):
            continue
        v = dict(zip(header, cells)).get("max_tokens_per_run", "-").replace(",", "").replace("_", "")
        if v.isdigit():
            caps[cells[0]] = int(v)
    return caps


def manager_today(today: str):
    try:
        s = json.loads(SESSIONS.read_text()) if SESSIONS.exists() else {}
    except ValueError:
        return None
    todays = [v for v in s.values() if v.get("last") == today]
    if not todays:
        return None
    return max(v["context"] for v in todays), sum(v["processed"] for v in todays), sum(v.get("cost_eq", 0) for v in todays)


def estimate(agent: str, history: list) -> int:
    seen = [int(r["tokens"]) for r in history if r["agent"] == agent and r.get("source") != "estimate"]
    if seen:
        return sum(seen) // len(seen)
    if agent.startswith("dev-") or agent in developers():
        return int(stack_json().get("estimates", {}).get("developer", DEVELOPER_ESTIMATE))
    return DEFAULT_ESTIMATES.get(agent, OTHER_ESTIMATE)


def main(argv: list) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd, args = argv[0], argv[1:]
    lim, history = limits(), rows()
    today = dt.date.today().isoformat()

    if cmd == "task":
        if not args:
            print(f"current task: {current_task()}")
            return 0
        CURRENT_TASK.parent.mkdir(parents=True, exist_ok=True)
        CURRENT_TASK.write_text(args[0] + "\n")
        print(f"agent runs are now booked against {args[0]}")
        return 0

    if cmd == "hook-subagent":
        try:
            data = json.load(sys.stdin)
        except ValueError:
            return 0
        path = data.get("agent_transcript_path") or ""
        det = transcript_detail(path) if path else {"ctx": 0}
        if not det["ctx"]:
            return 0
        ctx, processed, steps, start = det["ctx"], det["processed"], det["steps"], det["start"]
        now = dt.datetime.now()
        agent, aid = data.get("agent_type") or "agent", data.get("agent_id", "?")
        row = {"date": now.date().isoformat(), "time": now.strftime("%H:%M"), "task": prompt_task(path) or current_task(),
               "agent": agent, "tokens": ctx, "processed": processed, "source": "auto",
               "note": f"{steps} steps, prompt {first_prompt_chars(path) // 4} tok, id {aid}",
               "start": start, "session": data.get("session_id", ""), "start_cached": det["start_cached"],
               "cost_eq": det["cost_eq"]}
        old = rows()
        same = [i for i, r in enumerate(old) if r["source"] == "auto" and r["note"].endswith(f"id {aid}")]
        if same:  # an agent continued with SendMessage: its transcript keeps growing, so update its row instead of adding one
            row["task"] = old[same[-1]]["task"]
            row["start"] = old[same[-1]].get("start") or start
            row["start_cached"] = old[same[-1]].get("start_cached") or det["start_cached"]
            old[same[-1]] = row
            with LEDGER.open("w", newline="") as f:
                w = csv.DictWriter(f, fieldnames=FIELDS, extrasaction="ignore")
                w.writeheader()
                w.writerows(old)
        else:
            write_row(row)
        return 0

    if cmd == "hook-session":
        try:
            data = json.load(sys.stdin)
        except ValueError:
            return 0
        det = transcript_detail(data.get("transcript_path") or "")
        ctx, processed, steps = det["ctx"], det["processed"], det["steps"]
        if not steps:
            return 0
        try:
            sessions = json.loads(SESSIONS.read_text()) if SESSIONS.exists() else {}
        except ValueError:
            sessions = {}
        sid = data.get("session_id") or "session"
        prev = sessions.get(sid, {})
        first = prev.get("date", today)
        sessions[sid] = {**{k: v for k, v in prev.items() if k.startswith("warned_")}, "date": first, "last": today, "context": ctx, "processed": processed, "steps": steps,
                         "cost_eq": det["cost_eq"]}
        cutoff = (dt.date.today() - dt.timedelta(days=30)).isoformat()
        sessions = {k: v for k, v in sessions.items() if v.get("last", "") >= cutoff}
        # Tell the OWNER (not the manager) when the session gets expensive: a long manager conversation is re-sent on
        # every step, and the plan allowance is the real limit. Shown once per level, as a Claude Code system message.
        msgs, me = [], sessions[sid]
        clear_at = int(team_value("manager_clear_at", "150,000").replace(",", "").replace("_", "") or 150000)
        level = (ctx // 50_000) * 50_000
        if ctx >= clear_at and level > int(me.get("warned_ctx", 0)):
            me["warned_ctx"] = level
            msgs.append(f"Manager context is {ctx // 1000}k (limit {clear_at // 1000}k): every step re-sends all of it. "
                        "Good moment for: just milestone \"<where things stand>\" and then /clear.")
        u = plan_usage()
        for key, label in (("five_hour", "5-hour"), ("seven_day", "7-day")):
            pct = float((u.get(key) or {}).get("used_percentage") or 0)
            step = max((t for t in (70, 85, 95) if pct >= t), default=0)
            if step and step > int(me.get(f"warned_{key}", 0)):
                me[f"warned_{key}"] = step
                msgs.append(f"Plan usage: {pct:.0f}% of your {label} allowance used. "
                            + ("Finish the current step and pause new launches." if step >= 95 else
                               "Prefer continuing warm agents and one-call reviews; postpone optional work."))
        SESSIONS.parent.mkdir(parents=True, exist_ok=True)
        SESSIONS.write_text(json.dumps(sessions, indent=1) + "\n")
        if msgs:
            print(json.dumps({"systemMessage": " ".join(msgs)}))
        return 0

    if cmd == "add":
        estimate_flag = "--estimate" in args
        args = [a for a in args if a != "--estimate"]
        if len(args) < 3 or not args[2].replace(",", "").isdigit():
            sys.exit("usage: budget.py add <task> <agent> <tokens> [--estimate] [note]")
        now = dt.datetime.now()
        write_row({"date": now.date().isoformat(), "time": now.strftime("%H:%M"), "task": args[0],
                   "agent": args[1], "tokens": int(args[2].replace(",", "")), "note": " ".join(args[3:]),
                   "source": "estimate" if estimate_flag else "manual", "processed": ""})
        history = rows()
        cap = agent_caps().get(args[1])
        used = int(args[2].replace(",", ""))
        over = cap is not None and used > cap
        if over:
            print(f"OVER AGENT CAP: {args[1]} used {fmt(used)}, its max_tokens_per_run is {fmt(cap)}. "
                  f"Tell the user; give its next launch a smaller scope, or ask the user to raise the cap.")
        args = [args[0]]
        cmd = "status"

    if cmd == "status":
        task = args[0] if args else None
        day_used = used_by(history, lambda r: r["date"] == today, today)
        parts = []
        if task:
            used = used_by(history, lambda r: r["task"] == task)
            parts.append(usage_line(f"task {task}", used, lim["per_task_tokens"], lim["usd_per_million_tokens"]))
        parts.append(usage_line("today", day_used, lim["per_day_tokens"], lim["usd_per_million_tokens"]))
        est = sum(int(r["tokens"]) for r in history if r["date"] == today and r["source"] == "estimate")
        if est:
            parts.append(f"of which estimates {fmt(est)}")
        mgr = manager_today(today)
        if mgr:
            parts.append(f"manager {fmt(mgr[0])} context, {fmt(mgr[1])} processed today")
        pu = plan_usage_line()
        if pu:
            parts.append(pu)
        print(" | ".join(parts))
        return 0

    if cmd == "starts":
        # typical cold-start size per agent type: what to slim (preloaded skills, CLAUDE.md, memory)
        by = {}
        for r in history:
            if r["start"]:
                by.setdefault(r["agent"], []).append((int(r["start"]), int(r.get("start_cached") or 0), bool(r.get("start_cached"))))
        if not by:
            print("no measured starts yet (runs recorded by the budget hook after this kit version)")
            return 0
        print(f"{'agent':20} {'runs':>4} {'avg start':>9} {'max':>8} {'cached':>7} {'start cost-eq':>13}")
        for agent, xs in sorted(by.items(), key=lambda x: -sum(v[0] for v in x[1]) / len(x[1])):
            avg = sum(v[0] for v in xs) // len(xs)
            known = [v for v in xs if v[2] or v[1]]
            cached = (sum(v[1] for v in known) * 100 // max(sum(v[0] for v in known), 1)) if known else None
            ceq = sum(v[1] * 0.1 + (v[0] - v[1]) * 1.25 for v in known) // len(known) if known else None
            print(f"{agent:20} {len(xs):>4} {fmt(avg):>9} {fmt(max(v[0] for v in xs)):>8} "
                  f"{(str(cached) + '%') if cached is not None else '-':>7} {fmt(ceq) if ceq is not None else '-':>13}")
        print("start = Claude Code's own prompt + CLAUDE.md + the agent file + preloaded skills + skill list + memory.")
        print("cached = share of the start read from the prompt cache (paid at ~10%); the rest is written to the cache (x1.25).\n"
              "Starts of the same agent type within ~5 minutes share their cached prefix; spread-out launches pay it in full.")
        return 0

    if cmd == "report":
        task = args[0] if args else current_task()
        mine = [r for r in history if r["task"] == task]
        if not mine:
            print(f"no runs booked for {task}")
            return 0
        by = {}
        for r in mine:
            a = by.setdefault(r["agent"], [0, 0, 0, set(), 0, 0, 0])
            a[0] += 1
            a[1] += int(r["tokens"])
            a[2] += int(r["processed"] or 0)
            a[3].add(r["source"])
            a[4] += int(r["start"] or 0)
            a[5] += int(r.get("start_cached") or 0)
            a[6] += int(r.get("cost_eq") or 0)
        print(f"{'agent':20} {'runs':>4} {'tokens':>9} {'start':>8} {'cached':>7} {'work':>8} {'processed':>10} {'cost-eq':>8}  source")
        for agent, (n, tok, proc, src, st, sc, ceq) in sorted(by.items(), key=lambda x: -x[1][1]):
            work = f"{fmt(tok - st):>8}" if st else f"{'-':>8}"
            cached = f"{sc * 100 // st}%" if st and sc else "-"
            print(f"{agent:20} {n:>4} {fmt(tok):>9} {fmt(st) if st else '-':>8} {cached:>7} {work} "
                  f"{fmt(proc) if proc else '-':>10} {fmt(ceq) if ceq else '-':>8}  {','.join(sorted(src))}")
        total, total_start = sum(v[1] for v in by.values()), sum(v[4] for v in by.values())
        total_cached, total_ceq = sum(v[5] for v in by.values()), sum(v[6] for v in by.values())
        print(f"{'total':20} {len(mine):>4} {fmt(total):>9} {fmt(total_start) if total_start else '-':>8} "
              f"{(str(total_cached * 100 // total_start) + '%') if total_start and total_cached else '-':>7} {'':>8} {'':>10} "
              f"{fmt(total_ceq) if total_ceq else '-':>8}")
        if total_start:
            print(f"cold starts: {fmt(total_start)} of {fmt(total)} ({100 * total_start // max(total, 1)}%; target < 25%) - "
                  "each fresh launch pays its start again; continuing an agent doesn't")
            if total_cached:
                print(f"  of the starts, {100 * total_cached // total_start}% came from the prompt cache (~10% of the price); "
                      "cost-eq = plain-input-token equivalent (cache reads x0.1, writes x1.25, output x5)")
        mgr = manager_today(today)
        if mgr:
            print(f"manager session(s) today: {fmt(mgr[0])} context now, {fmt(mgr[1])} processed"
                  + (f", cost-eq {fmt(mgr[2])}" if mgr[2] else "")
                  + " - the manager re-sends its whole context on every step; a long session is usually the biggest cost")
        return 0

    if cmd == "check":
        if len(args) < 2:
            sys.exit("usage: budget.py check <task> <agent>:<count> ...   e.g. just budget-check F3 dev-core:2 code-reviewer:1")
        task, plan = args[0], args[1:]
        projected = 0
        for item in plan:
            agent, _, count = item.partition(":")
            est = estimate(agent, history)
            cap = agent_caps().get(agent)
            projected += (min(est, cap) if cap else est) * int(count or 1)
        task_used = used_by(history, lambda r: r["task"] == task)
        day_used = used_by(history, lambda r: r["date"] == today, today)
        worst, notes = 0, []
        for label, used, limit in (("task", task_used, lim["per_task_tokens"]), ("today", day_used, lim["per_day_tokens"])):
            if not limit:
                continue
            pct = (used + projected) * 100 // limit
            notes.append(f"{label} {fmt(used)} + ~{fmt(projected)} = {pct}% of {fmt(limit)}")
            if pct >= 100:
                worst = max(worst, 2)
            elif pct >= lim["warn_percent"]:
                worst = max(worst, 1)
        # the owner's real plan allowance outranks the kit's own day cap
        for key, label in (("five_hour", "5-hour plan window"), ("seven_day", "7-day plan window")):
            pct = float((plan_usage().get(key) or {}).get("used_percentage") or 0)
            if pct:
                tier = (" - stop and ask the user" if pct >= 95 else " - finish the current slice, plan the rest with the user"
                        if pct >= 85 else " - tell the user" if pct >= 70 else "")
                notes.append(f"{label} {pct:.0f}% used{tier}")
                worst = max(worst, 2 if pct >= 95 else 1 if pct >= 70 else 0)
        age = plan_usage_age()
        if age is None:
            notes.append("real plan usage unknown (the kit status line records it; `just doctor` checks it is installed)")
        elif age > 60:
            notes.append(f"real plan usage is {age} min old - treat it as a lower bound")
        verdict = {0: "OK", 1: "WARN (tell the user)", 2: "OVER LIMIT (ask the user before launching)"}[worst]
        print(f"{verdict}: " + ("; ".join(notes) if notes else f"no limits set, projected ~{fmt(projected)}"))
        return worst

    sys.exit(f"unknown command {cmd!r}; see --help")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
