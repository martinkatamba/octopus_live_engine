import json
import sys
import time
import subprocess
import urllib.request
import urllib.error

def trigger(scenario: str, port: int = 8000) -> str:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    if scenario == "login_401":
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/api/v1.0.0/customer_auth_portal/auth/login",
            data=json.dumps({"username": "sarah_vip", "password": "WRONG_PASSWORD"}).encode(),
            headers={"Content-Type": "application/json"},
            method="POST"
        )
    elif scenario == "crash_500":
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/api/v1.0.0/customer_auth_portal/auth/login",
            data=json.dumps({"username": "crash_user", "password": "password123"}).encode(),
            headers={"Content-Type": "application/json"},
            method="POST"
        )
    elif scenario == "guard_401":
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/api/v1.0.0/customer_auth_portal/profile",
            headers={},
            method="GET"
        )
    else:
        raise ValueError(f"Unknown scenario: {scenario}")

    try:
        resp = opener.open(req)
        return resp.headers.get("X-Job-ID", "")
    except urllib.error.HTTPError as e:
        return e.headers.get("X-Job-ID", "")

def resolve_job(job_id: str, port: int = 8000) -> str:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/api/visualizer/resolve-job?job_id={job_id}",
        method="GET"
    )
    try:
        resp = opener.open(req)
        return resp.read().decode()
    except Exception as e:
        return str(e)

def capture_screenshot(url: str, output_path: str) -> None:
    chrome_bin = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    cmd = [
        chrome_bin,
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--window-size=1920,1080",
        "--virtual-time-budget=4000",
        f"--screenshot={output_path}",
        url
    ]
    subprocess.run(cmd, check=True)

if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "trigger"
    if action == "trigger":
        scen = sys.argv[2] if len(sys.argv) > 2 else "login_401"
        port_arg = int(sys.argv[3]) if len(sys.argv) > 3 else 8000
        print(trigger(scen, port_arg))
    elif action == "resolve":
        jid = sys.argv[2]
        port_arg = int(sys.argv[3]) if len(sys.argv) > 3 else 8000
        print(resolve_job(jid, port_arg))
    elif action == "screenshot":
        target_url = sys.argv[2]
        out_file = sys.argv[3]
        capture_screenshot(target_url, out_file)
        print(f"Saved snapshot to {out_file}")
