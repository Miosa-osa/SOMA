"""Where the EPT is thrown away during a resume, against the machine s own milestones."""
import collections, glob, json, os, re, sys
label = sys.argv[1]; out = "/srv/soma/ept/raw"
tl = json.load(open(sorted(glob.glob(f"{out}/{label}.tl/*.json"), key=os.path.getmtime)[-1]))["milestones_ns"]
run = tl["RunStart"]; marks = {k: (v - run) / 1e6 for k, v in tl.items()}
rows = []
for line in open(f"{out}/{label}.txt"):
    m = re.match(r"\s+(\S+)\s+(\d+)\s+\[\d+\]\s+([\d.]+):\s+(\S+):\s*(.*)", line)
    if m: rows.append((float(m.group(3)), m.group(4).rstrip(":"), m.group(5).strip(), m.group(2), m.group(1)))
rows.sort()
tid = collections.Counter(r[3] for r in rows if r[4].startswith("soma-kvm-vcpu")).most_common(1)[0][0]
t0 = [r for r in rows if r[3] == tid][0][0]
keep = ("RunStart", "LaunchPageConsumed", "VsockConnected", "Handshake", "LaunchPageRetired", "Ready")
print("milestones, ms from RunStart:")
for k in keep:
    if k in marks: print("   %-20s %8.2f" % (k, marks[k]))
print()
print("kvm_mmu_zap_all_fast, ms from RunStart:")
for r in rows:
    if r[1] == "kvmmmu:kvm_mmu_zap_all_fast" and -1 < (r[0] - t0) * 1000 < marks["Ready"] + 5:
        print("   %8.2f  thread %s" % ((r[0] - t0) * 1000, r[4]))
print()
print("EPT violations per millisecond of the resume:")
f = [r for r in rows if r[3] == tid and r[1] == "kvm:kvm_page_fault" and (r[0] - t0) * 1000 <= marks["Ready"]]
per = collections.Counter(int((r[0] - t0) * 1000) for r in f)
for ms in range(int(marks["Ready"]) + 1):
    tag = "".join("   <- " + k for k in keep if int(marks.get(k, -1)) == ms)
    print("   %3d ms %5d%s" % (ms, per[ms], tag))
