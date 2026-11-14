"""Open craftcloud.figcraft in the FigCraft *web* build (headless Chromium, WebGL2 on SwiftShader) and
screenshot a frame, to check the file works there too.

    python3 -I web_verify.py <chromium> <url-of-verify.html?page=..&frame=..> <out.png>

Talks to Chromium over --remote-debugging-pipe (CDP, NUL-separated JSON), so no Node, Playwright or
websocket library is needed. verify.html (next to this file) iframes `figcraft/index.html?control&webgl`,
loads the three fonts with `text.loadFont`, opens the file with `figcraft.openFile`, zooms to the frame
and sets document.title to "done" with the app's own `figcraft.screenshot()` drawn into the page.
"""

import base64
import json
import os
import subprocess
import sys
import tempfile
import time


def main():
    chromium, url, out = sys.argv[1:4]
    r_ours, w_chrome = os.pipe()  # chrome writes fd 4 -> we read
    r_chrome, w_ours = os.pipe()  # we write -> chrome reads fd 3
    prof = tempfile.mkdtemp(prefix="cc-chrome-")

    def child():
        # Move both ends above 4 first, so dup2 never maps an fd onto itself (which would keep CLOEXEC).
        a, b = os.dup(r_chrome), os.dup(w_chrome)
        os.dup2(a, 3)
        os.dup2(b, 4)

    p = subprocess.Popen(
        ["nice", "-n", "10", chromium, "--headless=new", "--no-sandbox", f"--user-data-dir={prof}",
         "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--window-size=1440,900",
         "--remote-debugging-pipe", "about:blank"],
        preexec_fn=child, close_fds=False,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(r_chrome)
    os.close(w_chrome)
    rf = os.fdopen(r_ours, "rb", buffering=0)
    wf = os.fdopen(w_ours, "wb", buffering=0)
    n = [0]
    buf = [b""]

    def send(method, params=None, session=None):
        n[0] += 1
        msg = {"id": n[0], "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        wf.write(json.dumps(msg).encode() + b"\0")
        while True:
            while b"\0" not in buf[0]:
                chunk = rf.read(1 << 20)
                if not chunk:
                    raise RuntimeError("chromium closed the pipe")
                buf[0] += chunk
            line, buf[0] = buf[0].split(b"\0", 1)
            m = json.loads(line)
            if m.get("id") == n[0]:
                if "error" in m:
                    raise RuntimeError(m["error"])
                return m["result"]

    try:
        tid = send("Target.createTarget", {"url": url})["targetId"]
        sid = send("Target.attachToTarget", {"targetId": tid, "flatten": True})["sessionId"]
        send("Page.enable", session=sid)
        t0 = time.time()
        title = ""
        while time.time() - t0 < 240:
            time.sleep(2)
            title = send("Runtime.evaluate", {"expression": "document.title"}, sid)["result"].get("value", "")
            if title in ("done", "fail"):
                break
        log = send("Runtime.evaluate", {"expression": "(document.getElementById('log')||{}).textContent||''"},
                   sid)["result"].get("value", "")
        print("title:", title, f"after {time.time() - t0:.0f}s")
        print(log)
        shot = send("Page.captureScreenshot", {"format": "png"}, sid)
        with open(out, "wb") as f:
            f.write(base64.b64decode(shot["data"]))
        print("wrote", out)
    finally:
        p.kill()


if __name__ == "__main__":
    main()
