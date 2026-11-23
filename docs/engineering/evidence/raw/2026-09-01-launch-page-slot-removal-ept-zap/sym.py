"""Names the guest instruction pointers that take a resume s EPT violations."""
import bisect, collections, re, subprocess, sys

vmlinux, path, window = sys.argv[1], sys.argv[2], float(sys.argv[3]) / 1000.0
syms = []
for line in subprocess.run(["nm", "-n", vmlinux], capture_output=True, text=True).stdout.splitlines():
    parts = line.split()
    if len(parts) == 3 and parts[1] in "tTwW":
        syms.append((int(parts[0], 16), parts[2]))
syms.sort()
addrs = [a for a, _ in syms]

def name(rip):
    i = bisect.bisect_right(addrs, rip) - 1
    if i < 0 or rip - addrs[i] > 0x10000:
        return None
    return f"{syms[i][1]}+0x{rip - addrs[i]:x}"

rows = []
for line in open(path):
    m = re.match(r"\s+(\S+)\s+(\d+)\s+\[\d+\]\s+([\d.]+):\s+(\S+):\s*(.*)", line)
    if m:
        rows.append((float(m.group(3)), m.group(4).rstrip(":"), m.group(5).strip(),
                     m.group(2), m.group(1)))
rows.sort()
tid = collections.Counter(r[3] for r in rows if r[4].startswith("soma-kvm-vcpu")).most_common(1)[0][0]
rows = [r for r in rows if r[3] == tid]
t0 = rows[0][0]

seen, faults = set(), []
for moment, kind, arg, _, _ in rows:
    if kind != "kvm:kvm_page_fault" or moment - t0 > window:
        continue
    m = re.search(r"rip (0x\S+) address (0x\S+) error_code (0x\S+)", arg)
    if not m:
        continue
    rip, gpa, err = int(m.group(1), 16), int(m.group(2), 16), int(m.group(3), 16)
    if (gpa >> 12, rip, err & 2) in seen:
        continue
    seen.add((gpa >> 12, rip, err & 2))
    faults.append((moment - t0, rip, gpa, err))

by_fn = collections.Counter()
for _, rip, _, _ in faults:
    n = name(rip)
    by_fn[n.split("+")[0] if n else ("user 0x%x" % rip if rip < 0xffff800000000000 else "unresolved")] += 1
print(f"{len(faults)} deduplicated faults, {len({g >> 12 for _, _, g, _ in faults})} distinct guest pages")
print("\nfaulting function, guest side:")
for fn, n in by_fn.most_common(30):
    print(f"  {n:6d}  {100*n/len(faults):5.1f}%  {fn}")
