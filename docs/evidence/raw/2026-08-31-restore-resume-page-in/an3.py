"""What the first KVM_RUN call and the rest of the resume actually spend their time on.

Reads a `perf script` dump of `kvm:kvm_entry` and `kvm:kvm_exit` for one vCPU thread and
separates guest execution from the exits KVM resolves in the kernel without returning to the
userspace loop.
"""

import collections
import re
import sys

path = sys.argv[1]
events = []
for line in open(path):
    match = re.match(r"\s+\S+\s+(\d+)\s+\[\d+\]\s+([\d.]+):\s+(\S+):\s*(.*)", line)
    if match:
        events.append((float(match.group(2)), match.group(3).rstrip(":"),
                       match.group(4).strip(), match.group(1)))
events.sort()
tid = collections.Counter(event[3] for event in events).most_common(1)[0][0]
events = [event for event in events if event[3] == tid]

USERSPACE = {"IO_INSTRUCTION", "EPT_MISCONFIG", "HLT", "SHUTDOWN"}


def reason_of(argument):
    found = re.search(r"reason (\S+)", argument)
    return found.group(1) if found else "?"


def anatomy(sequence, label):
    inside, entry = 0.0, None
    reasons = collections.Counter()
    for moment, name, argument, _ in sequence:
        if name == "kvm:kvm_entry":
            entry = moment
        else:
            reasons[reason_of(argument)] += 1
            if entry is not None:
                inside += moment - entry
    wall = sequence[-1][0] - sequence[0][0]
    print(f"{label}: wall={wall * 1000:.3f} ms  guest={inside * 1000:.3f} ms  "
          f"kernel_exit_handling={(wall - inside) * 1000:.3f} ms")
    print("   ", reasons.most_common())


first = []
for event in events:
    first.append(event)
    if event[1] == "kvm:kvm_exit" and reason_of(event[2]) in USERSPACE:
        break
anatomy(first, "first KVM_RUN call")
start = events[0][0]
for window in (0.005, 0.010, 0.030):
    anatomy([e for e in events if e[0] - start <= window],
            f"first {window * 1000:.0f} ms of the resume")
