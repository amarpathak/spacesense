#!/usr/bin/env python3
"""M1 golden check (PLAN.md §7): every catalog rule and project artifact size in
`spacesense report --json` vs `du -skx` on the same paths.

usage: bench/golden.py [report.json]   (default: runs the report)"""
import json, subprocess, sys, os

SS = os.path.join(os.path.dirname(__file__), "..", "target", "release", "spacesense")
if len(sys.argv) > 1:
    rep = json.load(open(sys.argv[1]))
else:
    rep = json.loads(subprocess.run([SS, "report", "--json", "--no-git"], capture_output=True, text=True, check=True).stdout)

def du(path):
    out = subprocess.run(["du", "-skx", path], capture_output=True, text=True).stdout.split()
    return int(out[0]) * 1024 if out else None

worst = 0.0
rows = []
for r in rep["rules"]:
    for p in r["paths"]:
        if p.get("error") or p["size"] < 50_000_000:
            continue
        d = du(p["path"])
        diff = (p["size"] - d) / d * 100 if d else 0
        worst = max(worst, abs(diff))
        rows.append((p["size"], d, diff, r["id"], p["path"]))

arts = [a for pr in rep["projects"] for a in pr["artifacts"] if a["name"] == "node_modules"]
ss_nm = sum(a["size"] for a in arts)
du_nm = sum(du(a["path"]) or 0 for a in arts)
rows.append((ss_nm, du_nm, (ss_nm - du_nm) / du_nm * 100, f"node_modules x{len(arts)}", "(project artifacts)"))
worst = max(worst, abs(rows[-1][2]))

print(f"{'spacesense':>12} {'du -skx':>12} {'diff':>7}  rule / path")
for s, d, diff, rid, path in sorted(rows, reverse=True):
    print(f"{s/1e9:10.2f}GB {d/1e9:10.2f}GB {diff:+6.2f}%  {rid}  {path.replace(os.path.expanduser('~'), '~')}")
print(f"\nworst difference: {worst:.2f}%  ({'OK' if worst <= 2 else 'FAIL'}, limit 2%)")
sys.exit(0 if worst <= 2 else 1)
