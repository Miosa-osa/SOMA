#!/usr/bin/env python3
"""Pairs moonlight-web with Sunshine once, when the desktop image is built, as the desktop user.

Moonlight pairs by a PIN a person types into Sunshine; a desktop that opens already paired cannot
ask anyone. So this starts the screen, Sunshine and moonlight-web inside the image build, asks
moonlight-web to pair (it answers with the PIN), hands the PIN to Sunshine's own pairing API,
makes moonlight-web's only user its default user (so the page opens without a login), and stops
everything. What persists is the two programs' own state files in the desktop user's home.

Every Instance restores the same bytes, so every desktop shares these keys. That is acceptable
because both ends listen in the same guest: Sunshine on loopback only, moonlight-web on the viewer
port, which only the platform's authenticated tunnel reaches. The passwords are random and thrown
away; nothing needs them again.
"""
import base64, http.client, json, os, secrets, ssl, subprocess, sys, time

HOME = os.environ["HOME"]
VIEWER = "127.0.0.1:8090"  # a build-time port; the running viewer binds its own
STATE = "/tmp/soma-viewer-pair"
os.makedirs(STATE, exist_ok=True)


def wait_port(port, what):
    for _ in range(300):
        try:
            http.client.HTTPConnection("127.0.0.1", port, timeout=1).connect()
            return
        except OSError:
            time.sleep(0.1)
    sys.exit(f"pair.py: {what} never listened on {port}")


def web(method, path, body=None, cookie=None):
    c = http.client.HTTPConnection(*VIEWER.split(":"), timeout=60)
    headers = {"content-type": "application/json"}
    if cookie:
        headers["cookie"] = cookie
    c.request(method, path, json.dumps(body) if body is not None else None, headers)
    return c.getresponse()


def lines(response):
    """moonlight-web streams some answers as one JSON document per line."""
    buf = b""
    while True:
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            if line.strip():
                yield json.loads(line)
        chunk = response.read1(4096)
        if not chunk:
            if buf.strip():
                yield json.loads(buf)
            return
        buf += chunk


TLS = ssl._create_unverified_context()  # Sunshine's own self-signed loopback certificate


def sunshine(method, path, auth, body=None, token=None):
    c = http.client.HTTPSConnection("127.0.0.1", 47990, context=TLS, timeout=30)
    headers = {"authorization": "Basic " + auth, "content-type": "application/json"}
    if token:
        headers["X-CSRF-Token"] = token
    c.request(method, path, json.dumps(body) if body is not None else None, headers)
    r = c.getresponse()
    return r.status, json.loads(r.read() or b"{}")


user, password = "soma", secrets.token_urlsafe(24)
auth = base64.b64encode(f"{user}:{password}".encode()).decode()
procs = []
try:
    procs.append(subprocess.Popen(
        ["/usr/lib/xorg/Xorg", ":0", "-config", "/etc/soma-desktop/xorg.conf", "-logfile", f"{STATE}/Xorg.log",
         "-nolisten", "tcp", "-noreset", "-novtswitch", "-sharevts", "-keeptty"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
    for _ in range(100):
        if os.path.exists("/tmp/.X11-unix/X0"):
            break
        time.sleep(0.1)
    env = dict(os.environ, DISPLAY=":0")
    subprocess.run(["sunshine", "--creds", user, password], env=env, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(subprocess.Popen(["sunshine", f"{HOME}/.config/sunshine/sunshine.conf"], env=env,
                                  stdout=open(f"{STATE}/sunshine.log", "w"), stderr=subprocess.STDOUT))
    wait_port(47990, "Sunshine")
    procs.append(subprocess.Popen(
        ["./web-server", "--config-path", f"{HOME}/.config/moonlight-web/config.json",
         "--bind-address", VIEWER, "run"], cwd="/opt/moonlight-web", env=env,
        stdout=open(f"{STATE}/moonlight-web.log", "w"), stderr=subprocess.STDOUT))
    wait_port(int(VIEWER.split(":")[1]), "moonlight-web")

    # The first login creates the admin user.
    r = web("POST", "/api/login", {"name": user, "password": password})
    r.read()
    assert r.status == 200, f"login answered {r.status}"
    cookie = r.getheader("set-cookie").split(";")[0]

    # Sunshine's address as IPv4: "localhost" resolves to ::1 first, where Sunshine does not listen.
    r = web("POST", "/api/host", {"address": "127.0.0.1", "http_port": None}, cookie)
    body = r.read()
    assert r.status == 200, f"adding Sunshine answered {r.status}: {body[:200]!r}"
    host_id = json.loads(body)["host"]["host_id"]

    r = web("POST", "/api/pair", {"host_id": host_id}, cookie)
    answers = lines(r)
    pin = next(answers)["Pin"]
    pairing_id = None
    for _ in range(50):
        status, pending = sunshine("GET", "/api/pin", auth)
        if pending.get("pairings"):
            pairing_id = pending["pairings"][-1]["id"]
            break
        time.sleep(0.2)
    assert pairing_id, "Sunshine never saw the pairing request"
    token = sunshine("GET", "/api/csrf-token", auth)[1]["csrf_token"]
    status, saved = sunshine("POST", "/api/pin", auth, {"pin": pin, "name": "soma-viewer", "pairing_id": pairing_id}, token)
    assert status == 200 and saved.get("status") is True, f"Sunshine refused the PIN: {status} {saved}"
    result = next(answers)
    assert "Paired" in result, f"pairing did not complete: {result}"

    # The page opens as this user without a login.
    r = web("GET", "/api/user", None, cookie)
    me = json.loads(r.read())
    r = web("PUT", "/api/user/default", {"id": me["id"]}, cookie)
    r.read()
    assert r.status == 200, f"setting the default user answered {r.status}"
    print(f"pair.py: moonlight-web paired with Sunshine (host {host_id}), default user {me['id']}")
finally:
    for p in reversed(procs):
        p.terminate()
        try:
            p.wait(10)
        except subprocess.TimeoutExpired:
            p.kill()
