"""A second opinion on the sponsor accounts, with Daytona's own Python package.

`duet doctor` speaks to Daytona with Duet's own Rust client. When it fails on the day, this tells in
about a minute whether the account is the problem or our client is: it does the same kind of work
(a sandbox, a command, a file, a server on a port, a signed link, delete) through the official SDK,
and asks Nosana one question with plain urllib, the way Golden Sea did.

    scripts/run.sh check            # through uv, with the pinned SDK

Keys: DAYTONA_API_KEY (or ~/Secrets/Daytona/key), NOSANA_API_KEY (or ~/Secrets/Nosana/key),
NOSANA_LLM_URL and NOSANA_LLM_MODEL if they were handed out. Nothing is kept: the sandbox is deleted
at the end, and its lifetime is set so Daytona deletes it anyway after ten minutes.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.request
from pathlib import Path


def secret(env: str, file: str) -> str | None:
    value = os.environ.get(env, "").strip()
    if value:
        return value
    path = Path.home() / "Secrets" / file
    return path.read_text().strip() if path.exists() else None


class Steps:
    def __init__(self) -> None:
        self.failed = False

    def run(self, name: str, fn):
        started = time.perf_counter()
        try:
            out = fn()
        except Exception as e:  # every failure is shown as it came back
            ms = (time.perf_counter() - started) * 1000
            print(
                f"  FAIL  {name:<46} {ms:>7.0f} ms\n        {type(e).__name__}: {str(e)[:400]}"
            )
            self.failed = True
            raise
        ms = (time.perf_counter() - started) * 1000
        print(f"  ok    {name:<46} {ms:>7.0f} ms")
        return out


def fetch(url: str, headers: dict[str, str] | None = None) -> tuple[int, str]:
    req = urllib.request.Request(url, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, r.read(4000).decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read(4000).decode("utf-8", "replace")


def daytona_v2(key: str, steps: Steps) -> None:
    from daytona import Daytona, DaytonaConfig

    print("Daytona, new API (SDK 2.0), through the official package")
    with Daytona(DaytonaConfig(api_key=key)) as d:
        me = steps.run("key known to the new API", lambda: d.v2.current_api_key())
        print(
            f"        project {me.context.project_name} in {me.context.org_name} · may: {', '.join(me.project_permissions)}"
        )
        sb = steps.run(
            "sandbox created and running",
            lambda: d.v2.create(
                labels={"app": "duet", "lobby": "check"},
                lifecycle_policy={"timeout_seconds": 600, "on_timeout": "delete"},
                wait_seconds=60,
            ),
        )
        print(f"        sandbox {sb.id}, state {sb.state}")
        try:
            if sb.state != "started":
                steps.run("waited until running", lambda: sb.wait())
            r = steps.run(
                "command run (uname -m)",
                lambda: sb.exec("uname -m", timeout=30, execution_timeout=30),
            )
            print(f"        exit {r.exit_code}, printed {r.stdout.strip()!r}")
            local = Path(__file__)
            steps.run(
                "file uploaded",
                lambda: sb.fs.upload_file(str(local), "/tmp/duet-check.py"),
            )
            r = steps.run(
                "file read back",
                lambda: sb.exec("wc -c < /tmp/duet-check.py", timeout=30),
            )
            print(
                f"        {r.stdout.strip()} bytes in the sandbox, {local.stat().st_size} sent"
            )
            steps.run(
                "a server started on port 3000",
                lambda: sb.process.start(
                    "python3 -m http.server 3000 || busybox httpd -f -p 3000",
                    name="web",
                ),
            )
            time.sleep(1.5)
            link = steps.run(
                "signed link made",
                lambda: sb.ports.signed_preview_link(3000, expires_in_seconds=600),
            )
            status, body = steps.run(
                "link opened, skipping the warning page",
                lambda: fetch(link.url, {"X-Daytona-Skip-Preview-Warning": "true"}),
            )
            print(
                f"        {status}, {'a directory listing: the server answered' if 'Directory listing' in body or '<html' in body.lower() else body[:120]!r}"
            )
            status, body = fetch(link.url)
            warning = "Directory listing" not in body
            print(
                f"        as a browser opens it: {status}, "
                + (
                    "Daytona's warning page comes first. People would see it once; it can be turned off for the organization."
                    if warning
                    else "straight to the server, no warning page."
                )
            )
        finally:
            steps.run("sandbox deleted", lambda: d.v2.delete(sb, timeout=60))


def daytona_v0(key: str, steps: Steps) -> None:
    from daytona import Daytona, DaytonaConfig

    print("Daytona, original API (SDK 0.231), through the official package")
    with Daytona(DaytonaConfig(api_key=key)) as d:
        v0 = d.v0
        from daytona.v0 import CreateSandboxFromSnapshotParams  # type: ignore

        sb = steps.run(
            "sandbox created",
            lambda: v0.create(
                CreateSandboxFromSnapshotParams(
                    labels={"app": "duet", "lobby": "check"},
                    auto_stop_interval=0,
                    auto_delete_interval=0,
                ),
                timeout=90,
            ),
        )
        print(f"        sandbox {sb.id}")
        try:
            r = steps.run(
                "command run (uname -m)",
                lambda: sb.process.exec("uname -m", timeout=30),
            )
            print(f"        exit {r.exit_code}, printed {r.result.strip()!r}")
        finally:
            steps.run("sandbox deleted", lambda: v0.delete(sb))


def nosana(steps: Steps) -> None:
    url = os.environ.get("NOSANA_LLM_URL", "https://inference.nosana.com/v1").rstrip(
        "/"
    )
    key = secret("NOSANA_API_KEY", "Nosana/key")
    if not key and not os.environ.get("NOSANA_LLM_URL"):
        print("  skip  Nosana: no key (NOSANA_API_KEY or ~/Secrets/Nosana/key)")
        return
    print(f"Nosana at {url}, with plain urllib")
    headers = {
        "Content-Type": "application/json",
        **({"Authorization": f"Bearer {key}"} if key else {}),
    }
    model = os.environ.get("NOSANA_LLM_MODEL")
    if not model:
        listed = steps.run(
            "models listed",
            lambda: json.loads(
                urllib.request.urlopen(
                    urllib.request.Request(f"{url}/models", headers=headers), timeout=20
                ).read()
            ),
        )
        ids = [m.get("id") for m in listed.get("data", []) if m.get("available", True)]
        model = ids[0] if ids else "default"
    print(f"        model {model}")
    body = {
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "Reply with one JSON object and nothing else.",
            },
            {
                "role": "user",
                "content": 'Say hello in English and Japanese as {"en": "...", "ja": "..."}',
            },
        ],
        "temperature": 0,
        "max_tokens": 120,
        "chat_template_kwargs": {"enable_thinking": False},
    }

    def ask():
        req = urllib.request.Request(
            f"{url}/chat/completions",
            data=json.dumps(body).encode(),
            headers=headers,
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=90) as r:
            return json.loads(r.read())["choices"][0]["message"]["content"]

    text = steps.run("one question answered", ask)
    print(f"        {text.strip()[:200]!r}")


def main() -> int:
    steps = Steps()
    key = secret("DAYTONA_API_KEY", "Daytona/key")
    if not key:
        print("  skip  Daytona: no key (DAYTONA_API_KEY or ~/Secrets/Daytona/key)")
    else:
        for part in (daytona_v2, daytona_v0):
            try:
                part(key, steps)
            except Exception:  # noqa: BLE001 - already printed by Steps.run; go on to the next part
                print("        (stopped this part at the failure above)")
            print()
    try:
        nosana(steps)
    except Exception:  # noqa: BLE001 - already printed by Steps.run
        print("        (stopped Nosana at the failure above)")
    print()
    print(
        "Something failed above."
        if steps.failed
        else "Done. Nothing that was tried failed."
    )
    return 1 if steps.failed else 0


if __name__ == "__main__":
    sys.exit(main())
