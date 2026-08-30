#!/usr/bin/env python3
"""Boundary-matched Isorun measurement. Always destroys what it creates."""
import json, os, sys, time, urllib.request, concurrent.futures as cf

BASE = "https://run-us.isorun.ai"
KEY = os.environ["ISORUN_API_KEY"]
IMAGE = os.environ.get("ISORUN_IMAGE", "node:22")
CMD = os.environ.get("ISORUN_CMD", "/usr/local/bin/node --version")


def call(method, path, body=None, timeout=180):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Authorization", "Bearer " + KEY)
    if data:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read() or b"{}")


ORIGIN = time.monotonic_ns()


def one(i):
    """create -> exec -> destroy, returning the timings we can defend."""
    rec = {"i": i}
    rid = None
    try:
        t0 = time.monotonic_ns()
        rec["create_sent_ms"] = (t0 - ORIGIN) / 1e6
        run = call("POST", "/v1/runs", {
            "image": IMAGE, "vcpus": 1, "mem_mib": 1024,
            "disk_mib": 4096, "timeout": 300,
        })
        t1 = time.monotonic_ns()
        rid = run.get("id")
        rec["create_ms_server"] = run.get("create_ms")
        rec["create_ms_wall"] = (t1 - t0) / 1e6
        ex = call("POST", f"/v1/runs/{rid}/exec",
                  {"command": CMD, "timeout": 30})
        t2 = time.monotonic_ns()
        rec["exec_ms_wall"] = (t2 - t1) / 1e6
        rec["tti_ms_wall"] = (t2 - t0) / 1e6
        rec["exit_code"] = ex.get("exit_code")
        rec["stdout"] = ex.get("stdout", "")
        rec["ok"] = ex.get("exit_code") == 0 and ex.get("stdout", "").startswith("v")
    except Exception as e:                                    # noqa: BLE001
        rec["ok"] = False
        rec["error"] = f"{type(e).__name__}: {e}"
    finally:
        if rid:
            try:
                rec["destroy"] = call("DELETE", f"/v1/runs/{rid}")
            except Exception as e:                            # noqa: BLE001
                rec["destroy_error"] = str(e)
    return rec


def pct(values, p):
    """Nearest-rank percentile, the same rule the SOMA harness uses."""
    if not values:
        return None
    s = sorted(values)
    k = max(1, -(-p * len(s) // 100))
    return s[k - 1]


def main():
    n = int(sys.argv[1])
    conc = int(sys.argv[2])
    out = sys.argv[3]
    t0 = time.monotonic_ns()
    with cf.ThreadPoolExecutor(max_workers=conc) as pool:
        rows = list(pool.map(one, range(n)))
    wall = (time.monotonic_ns() - t0) / 1e6
    with open(out, "w") as fh:
        for r in rows:
            fh.write(json.dumps(r) + "\n")
    ok = [r for r in rows if r.get("ok")]
    srv = [r["create_ms_server"] for r in ok if r.get("create_ms_server") is not None]
    tti = [r["tti_ms_wall"] for r in ok]
    print(f"n={n} concurrency={conc} success={len(ok)}/{n} wall={wall:.0f} ms")
    sent = [r["create_sent_ms"] for r in rows if "create_sent_ms" in r]
    if sent:
        print(f"  request arrival spread: first {min(sent):.0f} ms, last {max(sent):.0f} ms "
              f"(a real burst needs this window small)")
    for name, vals in (("create_ms (server-reported)", srv), ("TTI wall from here", tti)):
        if vals:
            print(f"  {name:28} min {min(vals):7.1f}  p50 {pct(vals,50):7.1f}  "
                  f"p95 {pct(vals,95):7.1f}  p99 {pct(vals,99):7.1f}  max {max(vals):7.1f}")
    for r in rows:
        if not r.get("ok"):
            print("  FAILURE:", r.get("error", r))
    cost = sum((r.get("destroy") or {}).get("cost_cents", 0) for r in rows)
    print(f"  cost {cost:.4f} cents; every sandbox destroyed: "
          f"{all('destroy' in r for r in rows if r.get('i') is not None)}")


if __name__ == "__main__":
    main()
