"""What a restored guest s EPT violations are taken on, and by which guest code.

The window is the machine s own RunStart to Ready, taken from the timeline the same launch
wrote, so recording overhead lengthening the resume cannot silently shorten it.
"""
import bisect, collections, glob, json, os, re, subprocess, sys

label = sys.argv[1]
vmlinux = sys.argv[2] if len(sys.argv) > 2 else None
out = "/srv/soma/ept/raw"

timeline = json.load(open(sorted(glob.glob(f"{out}/{label}.tl/*.json"),
                                 key=os.path.getmtime)[-1]))["milestones_ns"]
window = (timeline["Ready"] - timeline["RunStart"]) / 1e9

rows = []
for line in open(f"{out}/{label}.txt"):
    m = re.match(r"\s+(\S+)\s+(\d+)\s+\[\d+\]\s+([\d.]+):\s+(\S+):\s*(.*)", line)
    if m:
        rows.append((float(m.group(3)), m.group(4).rstrip(":"), m.group(5).strip(),
                     m.group(2), m.group(1)))
rows.sort()
tid = collections.Counter(r[3] for r in rows
                          if r[4].startswith("soma-kvm-vcpu")).most_common(1)[0][0]
rows = [r for r in rows if r[3] == tid]
t0 = rows[0][0]
rows = [r for r in rows if r[0] - t0 <= window]

faults = []
for moment, kind, arg, _, _ in rows:
    if kind != "kvm:kvm_page_fault":
        continue
    m = re.search(r"rip (0x\S+) address (0x\S+) error_code (0x\S+)", arg)
    if m:
        faults.append((moment - t0, int(m.group(1), 16), int(m.group(2), 16),
                       int(m.group(3), 16)))
exits = collections.Counter(
    (re.search(r"reason (\S+)", a).group(1) if re.search(r"reason (\S+)", a) else "?")
    for _, k, a, _, _ in rows if k == "kvm:kvm_exit")
pages = {g >> 12 for _, _, g, _ in faults}
writes = sum(1 for _, _, _, e in faults if e & 2)

print(f"[{label}] RunStart to Ready {window*1000:.2f} ms")
print(f"  EPT violations              {exits["EPT_VIOLATION"]}")
print(f"  distinct guest pages        {len(pages)}  ({len(pages)*4/1024:.1f} MiB)")
print(f"  violations per page         {exits["EPT_VIOLATION"]/max(len(pages),1):.2f}")
print(f"  write faults / read faults  {writes} / {len(faults)-writes}")
print("  other exits                 " +
      ", ".join(f"{k} {v}" for k, v in exits.most_common() if k != "EPT_VIOLATION"))

per_page = collections.Counter(g >> 12 for _, _, g, _ in faults)
once = sum(1 for v in per_page.values() if v == 1)
twice = sum(1 for v in per_page.values() if v == 2)
more = sum(1 for v in per_page.values() if v > 2)
print(f"  pages faulted once/twice/3+ {once}/{twice}/{more}")
kinds = collections.Counter()
for page in pages:
    seen = {bool(e & 2) for _, _, g, e in faults if g >> 12 == page}
    kinds["read only" if seen == {False} else
          "write only" if seen == {True} else "read then write"] += 1
print("  by access:                  " + ", ".join(f"{k} {v}" for k, v in kinds.most_common()))

region = collections.Counter((g >> 26) for _, _, g, _ in faults)
print("  guest physical, 64 MiB regions above one percent:")
for k, v in sorted(region.items()):
    if v > len(faults) / 100:
        print(f"    {k*64:5d}-{(k+1)*64:5d} MiB  {v:5d}  {100*v/len(faults):5.1f}%")

if vmlinux:
    syms = sorted((int(p[0], 16), p[2]) for p in
                  (l.split() for l in subprocess.run(["nm", "-n", vmlinux],
                   capture_output=True, text=True).stdout.splitlines())
                  if len(p) == 3 and p[1] in "tTwW")
    addrs = [a for a, _ in syms]

    def name(rip):
        if rip < 0xffff800000000000:
            return "guest user code"
        i = bisect.bisect_right(addrs, rip) - 1
        return syms[i][1] if i >= 0 and rip - addrs[i] <= 0x10000 else "unresolved"
    by_fn = collections.Counter(name(r) for _, r, _, _ in faults)
    print("  faulting guest function, top 12:")
    for fn, n in by_fn.most_common(12):
        print(f"    {n:5d}  {100*n/len(faults):5.1f}%  {fn}")
    print("    distinct faulting instruction pointers: "
          f"{len({r for _, r, _, _ in faults})}")
