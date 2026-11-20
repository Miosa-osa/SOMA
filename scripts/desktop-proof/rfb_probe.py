#!/usr/bin/env python3
"""A viewer for a SOMA desktop, reached the way the platform reaches one: through the SOMA tunnel.

It opens `GET /v1/sandboxes/{id}/tunnel/6080` on soma-api's loopback face, speaks WebSocket to
KasmVNC inside the guest, then RFB 3.8 over it, exactly as a browser's noVNC client would, and
measures what a person looking at the desktop would see:

  first-frame   connect (retrying until the guest port answers) and time the first framebuffer
                update, then the first frame in which the desktop is painted (not one flat colour)
  click         N viewer clicks: right-click on the desktop, time until the menu is painted
                (pixels change under the pointer), then Escape and wait for it to close
  trigger       time from running a command (an agent's click through exec) to the paint

Every result is one JSON object on stdout; wall-clock stamps are nanoseconds since the epoch.
The protocol code is in viewer.py beside this file.
"""

import argparse, json, subprocess, sys, time

from viewer import ESCAPE, RIGHT, Direct, Rfb, Tunnel, WebSocket, now


def connect(args, deadline_s):
    started, attempts, last = now(), 0, ""
    while (now() - started) / 1e9 < deadline_s:
        attempts += 1
        try:
            if args.direct:
                tunnel = Direct(args.direct)
            else:
                tunnel = Tunnel(args.api, args.tenant, args.instance, args.port)
            opened = now()
            rfb = Rfb(WebSocket(tunnel))
            return rfb, {"probe_start_ns": started, "attempts": attempts, "tunnel_open_ns": opened,
                         "rfb_ready_ns": now(), "desktop": f"{rfb.width}x{rfb.height}", "name": rfb.name}
        except (OSError, ConnectionError) as error:
            last = str(error)
            time.sleep(0.05)
    raise SystemExit(json.dumps({"error": "viewer never connected", "attempts": attempts, "last": last}))


def first_frame(args):
    rfb, result = connect(args, args.deadline)
    rfb.request(incremental=False)
    rfb.update()
    result["first_update_ns"] = now()
    while not rfb.painted():
        if (now() - result["probe_start_ns"]) / 1e9 > args.deadline:
            raise SystemExit(json.dumps({**result, "error": "desktop never painted"}))
        rfb.request()
        rfb.update()
    result["first_painted_ns"] = now()
    if args.save:
        save_ppm(rfb, args.save)
    print(json.dumps(result))


def clicks(args):
    rfb, result = connect(args, 10)
    rfb.request(incremental=False)
    rfb.update()
    x, y = args.x, args.y
    region = (x, y, min(x + 160, rfb.width), min(y + 120, rfb.height))
    samples = []
    for _ in range(args.samples):
        rfb.pointer(x, y, 0)
        rfb.settle()
        sent = now()
        rfb.pointer(x, y, RIGHT)
        rfb.pointer(x, y, 0)
        painted = rfb.until_change(region, args.minimum)
        samples.append(None if painted is None else round((painted - sent) / 1e6, 3))
        rfb.key(ESCAPE)
        rfb.settle()
    result.update({"kind": "viewer_click_to_paint_ms", "point": [x, y], "samples": samples})
    print(json.dumps(result))


def trigger(args):
    rfb, result = connect(args, 10)
    rfb.request(incremental=False)
    rfb.update()
    rfb.settle()
    x, y = args.x, args.y
    region = (x, y, min(x + 160, rfb.width), min(y + 120, rfb.height))
    sent = now()
    child = subprocess.Popen(args.command, shell=True, stdout=subprocess.DEVNULL)
    painted = rfb.until_change(region, args.minimum, limit_s=15)
    child.wait()
    result.update({"kind": "trigger_to_paint_ms", "command_ms": round((now() - sent) / 1e6, 3),
                   "paint_ms": None if painted is None else round((painted - sent) / 1e6, 3)})
    rfb.key(ESCAPE)
    print(json.dumps(result))


def save_ppm(rfb, path):
    with open(path, "wb") as out:
        out.write(f"P6 {rfb.width} {rfb.height} 255\n".encode())
        fb = rfb.fb
        out.write(bytes(b for i in range(0, len(fb), 4) for b in (fb[i + 2], fb[i + 1], fb[i])))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("mode", choices=["first-frame", "click", "trigger"])
    parser.add_argument("--api", default="127.0.0.1:18930")
    parser.add_argument("--tenant", default="m1")
    parser.add_argument("--instance", default="")
    parser.add_argument("--direct", help="host:port of a viewer port reached without a tunnel")
    parser.add_argument("--port", type=int, default=6080)
    parser.add_argument("--deadline", type=float, default=30.0)
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--x", type=int, default=960)
    parser.add_argument("--y", type=int, default=540)
    parser.add_argument("--minimum", type=int, default=500)
    parser.add_argument("--save")
    parser.add_argument("--command")
    args = parser.parse_args()
    {"first-frame": first_frame, "click": clicks, "trigger": trigger}[args.mode](args)


if __name__ == "__main__":
    sys.exit(main())
