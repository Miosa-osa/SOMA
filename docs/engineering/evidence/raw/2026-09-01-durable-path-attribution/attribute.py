"""Where the time in one burst cohort actually goes, from the retained samples alone."""
import json, sys, statistics

def p50(v):
    return statistics.median(v)/1e6 if v else float("nan")

def run(path):
    ms = {k: [] for k in (
        "tti", "launch process", "  launch: resolution", "  launch: admission",
        "  launch: machine creation", "  launch: readiness", "  launch: process outside facade",
        "exec process", "  exec: dispatch", "  exec: execution", "  exec: process outside facade",
        "gap between the two processes", "destroy process", "  destroy: cleanup")}
    ok = 0
    for line in open(path):
        d = json.loads(line)
        if d.get("record_type") != "sample" or not d.get("successful"):
            continue
        ok += 1
        st = {op: {m["kind"]: m["elapsed_ns"] for m in mm}
              for op, mm in d["stages"].items()}
        pr = {op: v.get("duration_ns") for op, v in d["processes"].items()}
        def seg(op, a, b):
            m = st.get(op, {})
            return m[b] - m[a] if a in m and b in m else None
        lf = seg("launch", "accepted", "ready")
        ef = seg("exec", "accepted", "command_finished")
        ms["tti"].append(d["tti_ns"])
        ms["launch process"].append(pr["launch"])
        ms["  launch: resolution"].append(seg("launch", "accepted", "workload_resolved"))
        ms["  launch: admission"].append(seg("launch", "workload_resolved", "admitted"))
        ms["  launch: machine creation"].append(seg("launch", "admitted", "machine_launched"))
        ms["  launch: readiness"].append(seg("launch", "machine_launched", "ready"))
        ms["  launch: process outside facade"].append(pr["launch"] - lf)
        ms["exec process"].append(pr["exec"])
        ms["  exec: dispatch"].append(seg("exec", "accepted", "command_started"))
        ms["  exec: execution"].append(seg("exec", "command_started", "command_finished"))
        ms["  exec: process outside facade"].append(pr["exec"] - ef)
        ms["gap between the two processes"].append(d["tti_ns"] - pr["launch"] - pr["exec"])
        ms["destroy process"].append(pr["destroy"])
        ms["  destroy: cleanup"].append(seg("destroy", "cleanup_started", "cleanup_finished"))
    print(f"{path}  successful={ok}")
    for k, v in ms.items():
        v = [x for x in v if x is not None]
        print(f"  {k:38s} p50 {p50(v):9.2f} ms")

for path in sys.argv[1:]:
    run(path)
