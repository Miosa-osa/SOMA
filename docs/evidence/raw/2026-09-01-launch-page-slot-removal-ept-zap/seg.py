import sys, json
d = json.load(sys.stdin)
tl = d["receipt"]["milestones"]
t = {e["kind"]: e["elapsed_ns"] for e in tl}
print("%.2f %.2f" % ((t["ready"]-t["machine_launched"])/1e6, t["ready"]/1e6))
