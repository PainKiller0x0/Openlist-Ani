#!/usr/bin/env python3
"""Back up the legacy deployment and validate the Rust canary privately."""

import argparse
import hashlib
import base64
import hmac
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tarfile
import time
import tomllib
import re
import urllib.request
import urllib.error
from datetime import datetime, timezone

ROOT = Path("/opt/openlist-ani")


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def backup():
    destination = (
        ROOT
        / "backups"
        / ("pre-rust-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
    )
    destination.mkdir(mode=0o700)
    snapshots = destination / "data"
    snapshots.mkdir(mode=0o700)
    database_counts = {}
    for name in ("data.db", "task_mementos.db"):
        with sqlite3.connect(
            f'file:{ROOT / "data" / name}?mode=ro', uri=True
        ) as source:
            with sqlite3.connect(snapshots / name) as target:
                source.backup(target)
                if target.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
                    raise RuntimeError("database integrity check failed: " + name)
                database_counts[name] = {
                    table: target.execute(
                        'SELECT COUNT(*) FROM "' + table + '"'
                    ).fetchone()[0]
                    for (table,) in target.execute(
                        "SELECT name FROM sqlite_master WHERE type='table'"
                    )
                }
        os.chmod(snapshots / name, 0o600)
    for name in ("config.toml", "auth.env"):
        shutil.copy2(ROOT / name, destination / name)
        os.chmod(destination / name, 0o600)
    unit = Path("/etc/systemd/system/openlist-ani.service")
    shutil.copy2(unit, destination / "openlist-ani.service")
    dropins = Path("/etc/systemd/system/openlist-ani.service.d")
    if dropins.exists():
        shutil.copytree(dropins, destination / "openlist-ani.service.d")
    archive = destination / "legacy-runtime.tar.gz"
    with tarfile.open(archive, "w:gz") as target:
        for name in ("source", "venv", "bootstrap", "data/uploads"):
            if (ROOT / name).exists():
                target.add(ROOT / name, arcname=name)
    os.chmod(archive, 0o600)
    config = tomllib.loads((ROOT / "config.toml").read_text())
    subscriptions = config.get("rss", {}).get("subscriptions", [])
    manifest = {
        "backup": str(destination),
        "config_sha256": sha256(ROOT / "config.toml"),
        "unit_sha256": sha256(unit),
        "archive_sha256": sha256(archive),
        "database_counts": database_counts,
        "subscriptions": len(subscriptions),
        "active_subscriptions": sum(
            s.get("enabled", True) and not s.get("completed", False)
            for s in subscriptions
        ),
        "memory_before_bytes": int(
            subprocess.check_output(
                [
                    "systemctl",
                    "show",
                    "openlist-ani.service",
                    "-p",
                    "MemoryCurrent",
                    "--value",
                ],
                text=True,
            ).strip()
        ),
        "backend": config.get("backend", {}),
    }
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2))
    os.chmod(destination / "manifest.json", 0o600)
    (ROOT / "backups" / "rust-rollback-path").write_text(str(destination) + "\n")
    os.chmod(ROOT / "backups" / "rust-rollback-path", 0o600)
    print(json.dumps(manifest, indent=2))


def inspect():
    config = tomllib.loads((ROOT / "config.toml").read_text())
    safe = {}
    for section, fields in {
        "backend": ("host", "port"),
        "rss": ("interval_time", "max_download_retries"),
        "openlist": ("url", "download_path", "rename_format", "offline_download_tool"),
        "mikan": ("base_url",),
        "metadata_parser": ("provider",),
        "llm": ("tmdb_language", "openai_model"),
    }.items():
        safe[section] = {key: config.get(section, {}).get(key) for key in fields}
    safe["credential_configured"] = {
        section + "." + key: bool(config.get(section, {}).get(key))
        for section, key in [
            ("llm", "tmdb_api_key"),
            ("llm", "openai_api_key"),
            ("openlist", "token"),
        ]
    }
    safe["subscriptions"] = config.get("rss", {}).get("subscriptions", [])
    for item in safe["subscriptions"]:
        # Feed URLs may contain private query parameters; retain only public Mikan URLs.
        if "mikan" not in item.get("url", ""):
            item["url"] = "[private feed]"
    safe["task_states"] = {}
    with sqlite3.connect(
        f'file:{ROOT / "data/task_mementos.db"}?mode=ro', uri=True
    ) as connection:
        safe["task_states"] = connection.execute(
            "SELECT state, COUNT(*) FROM task_mementos GROUP BY state"
        ).fetchall()
    print(json.dumps(safe, ensure_ascii=False, indent=2))


def prepare():
    baseline = Path((ROOT / "backups/rust-rollback-path").read_text().strip())
    destination = ROOT / "rust-canary"
    destination.mkdir(mode=0o700, exist_ok=True)
    if not (destination / "data").exists():
        shutil.copytree(baseline / "data", destination / "data")
    source = (ROOT / "config.toml").read_text()
    section = re.search(r"(?ms)^\[backend\].*?(?=^\[|\Z)", source)
    if not section:
        raise RuntimeError("backend section missing")
    backend, count = re.subn(
        r"(?m)^port\s*=\s*\d+", "port = 27666", section.group(), count=1
    )
    if count != 1:
        raise RuntimeError("backend port missing")
    source = source[: section.start()] + backend + source[section.end() :]
    (destination / "config.toml").write_text(source)
    os.chmod(destination / "config.toml", 0o600)
    print("Prepared isolated canary configuration and SQLite copies on 127.0.0.1:27666")


def session_cookie():
    username = os.environ["OP_ANI_AUTH_USER"]
    secret = (
        os.environ.get("OP_ANI_AUTH_SECRET") or os.environ["OP_ANI_AUTH_PASSWORD_HASH"]
    )
    payload = username + "|" + str(int(time.time()) + 3600) + "|rust-live-acceptance"
    signature = (
        base64.urlsafe_b64encode(
            hmac.new(secret.encode(), payload.encode(), hashlib.sha256).digest()
        )
        .decode()
        .rstrip("=")
    )
    return payload + "." + signature


def request(base, path, cookie, body=None):
    req = urllib.request.Request(
        base + path,
        headers={
            "Cookie": "op_ani_session=" + cookie,
            "Content-Type": "application/json",
        },
    )
    if body is not None:
        req.data = json.dumps(body).encode()
    try:
        with urllib.request.urlopen(req, timeout=90) as response:
            raw = response.read()
            return response.status, (
                json.loads(raw)
                if response.headers.get("Content-Type", "").startswith(
                    "application/json"
                )
                else len(raw)
            )
    except urllib.error.HTTPError as error:
        return error.code, {"error": "HTTP " + str(error.code)}


def probe():
    cookie = session_cookie()
    old = "http://127.0.0.1:26666"
    new = "http://127.0.0.1:27666"
    report = {}
    for path in [
        "/",
        "/login",
        "/assets/js/app.js",
        "/healthz",
        "/readyz",
        "/api/auth/session",
        "/api/ui/state",
        "/api/ui/settings",
        "/api/downloads",
        "/api/ui/logs?limit=10",
    ]:
        status, result = request(new, path, cookie)
        if status != 200:
            raise RuntimeError(path + " failed: HTTP " + str(status))
        report[path] = {"status": status}
        if path == "/api/ui/state":
            old_status, original = request(old, path, cookie)
            if old_status != 200:
                raise RuntimeError("legacy authenticated state unavailable")
            keys = [
                "url",
                "name",
                "anime_name",
                "download_directory_name",
                "season",
                "episode_offset",
                "filename_season_override",
                "directory_season_override",
                "completed",
                "enabled",
                "tmdb_id",
                "poster_url",
                "latest_episode",
            ]
            for original_item, migrated in zip(
                original["rss_subscriptions"], result["rss_subscriptions"]
            ):
                differences = [
                    key for key in keys if original_item.get(key) != migrated.get(key)
                ]
                if differences:
                    raise RuntimeError(
                        "subscription mismatch: "
                        + original_item.get("name", "")
                        + " fields="
                        + ",".join(differences)
                    )
            assert len(original["rss_subscriptions"]) == len(
                result["rss_subscriptions"]
            )
            report[path]["subscriptions"] = len(result["rss_subscriptions"])
            report[path]["active_feeds"] = len(result["rss_urls"])
    config = tomllib.loads((ROOT / "config.toml").read_text())
    for subscription in config["rss"]["subscriptions"]:
        if subscription.get("completed") or not subscription.get("enabled", True):
            continue
        data = {
            key: subscription[key]
            for key in [
                "url",
                "name",
                "anime_name",
                "download_directory_name",
                "season",
                "episode_offset",
                "filename_season_override",
                "directory_season_override",
                "tmdb_id",
                "poster_url",
            ]
            if key in subscription
        }
        data["exclude_patterns"] = "|".join(subscription.get("exclude_patterns", []))
        status, result = request(new, "/api/ui/rss/preview", cookie, data)
        if status != 200 or not result.get("success"):
            raise RuntimeError("preview failed: " + subscription["name"])
        report[subscription["name"]] = {
            "total": result["total"],
            "included": result["included"],
            "excluded": result["excluded"],
            "examples": [
                {
                    key: entry.get(key)
                    for key in [
                        "title",
                        "season",
                        "episode",
                        "rename_preview",
                        "download_directory",
                    ]
                }
                for entry in result["entries"][:2]
            ],
        }
    storage = {
        "cookies": [
            {
                "name": "op_ani_session",
                "value": cookie,
                "domain": "127.0.0.1",
                "path": "/",
                "httpOnly": True,
                "secure": False,
                "sameSite": "Lax",
            }
        ],
        "origins": [],
    }
    session = ROOT / "rust-canary/browser-session.json"
    session.write_text(json.dumps(storage))
    os.chmod(session, 0o600)
    print(json.dumps(report, ensure_ascii=False, indent=2))


def external():
    cookie = session_cookie()
    base = "http://127.0.0.1:27666"
    checks = [
        ("/api/ui/mikan/search", {"keyword": "学生会"}),
        ("/api/ui/mikan/groups", {"bangumi_id": 4064}),
        ("/api/ui/mikan/rss", {"bangumi_id": 4064, "subgroup_id": 583}),
        (
            "/api/ui/rss/metadata",
            {
                "url": "https://mikanani.kas.pub/RSS/Bangumi?bangumiId=4064&subgroupid=583",
                "tmdb_id": 283938,
            },
        ),
    ]
    for endpoint, body in checks:
        start = time.monotonic()
        status, result = request(base, endpoint, cookie, body)
        if status != 200 or not result.get("success"):
            raise RuntimeError(
                "external adapter failed: " + endpoint + " HTTP " + str(status)
            )
        output = {
            "endpoint": endpoint,
            "status": status,
            "seconds": round(time.monotonic() - start, 2),
        }
        for key in ["name", "tmdb_id", "rss_url"]:
            if key in result:
                output[key] = result[key]
        if "results" in result:
            output["results"] = len(result["results"])
            assert output["results"] > 0
        if "groups" in result:
            output["groups"] = len(result["groups"])
            assert output["groups"] > 0
        print(json.dumps(output, ensure_ascii=False), flush=True)


def download(resume=False):
    cookie = session_cookie()
    base = "http://127.0.0.1:27666"
    path = ROOT / "rust-canary/config.toml"
    original = path.read_text()
    config = tomllib.loads(original)
    root_path = (
        config["openlist"]["download_path"].rstrip("/") + "/.rust-live-acceptance"
    )
    if resume:
        with sqlite3.connect(ROOT / "rust-canary/data/task_mementos.db") as connection:
            task_id, raw = next(
                (identifier, payload)
                for identifier, payload in connection.execute(
                    "SELECT task_id,payload FROM task_mementos"
                )
                if json.loads(payload).get("release", {}).get("title")
                == "Rust Acceptance S01E01"
            )
        root_path = json.loads(raw)["base_path"]
    source = original.replace(
        "download_path = "
        + json.dumps(config["openlist"]["download_path"], ensure_ascii=False),
        "download_path = " + json.dumps(root_path, ensure_ascii=False),
        1,
    )
    section = re.search(r"(?ms)^\[smartstrm\].*?(?=^\[|\Z)", source)
    if section:
        source = (
            source[: section.start()]
            + re.sub(r"(?m)^enabled\s*=\s*true", "enabled = false", section.group())
            + source[section.end() :]
        )
    path.write_text(source)
    assert tomllib.loads(source)["openlist"]["download_path"] == root_path
    try:
        if not resume:
            feed = next(
                item
                for item in config["rss"]["subscriptions"]
                if item.get("tmdb_id") == 283938
            )
            status, parsed = request(
                base, "/api/parse_rss", cookie, {"url": feed["url"], "limit": 1}
            )
            assert status == 200 and parsed["entries"]
            url = parsed["entries"][0]["download_url"]
            status, result = request(
                base,
                "/api/downloads",
                cookie,
                {"title": "Rust Acceptance S01E01", "download_url": url},
            )
            if status != 200 or not result.get("success"):
                raise RuntimeError(
                    "real OpenList submission failed: HTTP " + str(status)
                )
            task_id = result["task"]["id"]
        else:
            status, result = request(
                base, "/api/ui/downloads/" + task_id + "/retry", cookie, {}
            )
            if status != 200 or not result.get("success"):
                raise RuntimeError("checkpoint retry failed: HTTP " + str(status))
        print(
            json.dumps({"accepted": True, "task_id": task_id, "root": root_path}),
            flush=True,
        )
        database = ROOT / "rust-canary/data/task_mementos.db"
        deadline = time.monotonic() + 240
        state = None
        while time.monotonic() < deadline:
            with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
                row = connection.execute(
                    "SELECT state, provider_task_id FROM rust_download_operations WHERE task_id=?",
                    (task_id,),
                ).fetchone()
                record = connection.execute(
                    "SELECT state,payload FROM task_mementos WHERE task_id=?",
                    (task_id,),
                ).fetchone()
            if row and row[0] != state:
                state = row[0]
                print(json.dumps({"operation": state}), flush=True)
            if row and row[0] == "completed":
                break
            if record and record[0] == "failed":
                raise RuntimeError(
                    "download failed: "
                    + json.loads(record[1])
                    .get("retry", {})
                    .get("last_error", "unknown")
                )
            time.sleep(3)
        if state != "completed":
            raise RuntimeError("download did not complete in the acceptance window")
        target_directory = root_path + "/Rust Acceptance/Season 1"
        token = config["openlist"]["token"]

        def openlist_request(endpoint, body):
            req = urllib.request.Request(
                config["openlist"]["url"].rstrip("/") + endpoint,
                data=json.dumps(body).encode(),
                headers={"Authorization": token, "Content-Type": "application/json"},
            )
            with urllib.request.urlopen(req, timeout=30) as response:
                data = json.load(response)
            if data.get("code") != 200:
                raise RuntimeError("OpenList validation failed: " + endpoint)
            return data.get("data")

        files = openlist_request(
            "/api/fs/list",
            {
                "path": target_directory,
                "page": 1,
                "per_page": 0,
                "refresh": True,
                "password": "",
            },
        )["content"]
        video = next(
            item
            for item in files
            if item["name"].startswith("Rust Acceptance - S01E01.")
            and not item["is_dir"]
        )
        if video["size"] <= 0:
            raise RuntimeError("materialized file is empty")
        print(
            json.dumps(
                {
                    "completed": True,
                    "path": target_directory + "/" + video["name"],
                    "size": video["size"],
                    "real_openlist_file_verified": True,
                }
            ),
            flush=True,
        )
        parent, name = root_path.rsplit("/", 1)
        assert name == ".rust-live-acceptance"
        openlist_request("/api/fs/remove", {"dir": parent, "names": [name]})
        print(
            "Removed the isolated acceptance download; production library untouched",
            flush=True,
        )
    finally:
        path.write_text(original)


def download_status():
    config = tomllib.loads((ROOT / "rust-canary/config.toml").read_text())
    with sqlite3.connect(
        f'file:{ROOT / "rust-canary/data/task_mementos.db"}?mode=ro', uri=True
    ) as connection:
        rows = connection.execute("SELECT state,payload FROM task_mementos").fetchall()
    for state, raw in rows:
        payload = json.loads(raw)
        if (
            not payload.get("release", {})
            .get("title", "")
            .startswith("Rust Acceptance")
        ):
            continue
        remote = payload.get("downloader", {}).get("payload", {})
        print(
            json.dumps(
                {
                    "state": state,
                    "workflow": remote.get("workflow_state"),
                    "last_error": payload.get("retry", {}).get("last_error"),
                },
                ensure_ascii=False,
            )
        )
        plan = remote.get("materialization_plan", {})
        for directory in [plan.get("source_directory"), plan.get("target_directory")]:
            if not directory:
                continue
            req = urllib.request.Request(
                config["openlist"]["url"].rstrip("/") + "/api/fs/list",
                data=json.dumps(
                    {
                        "path": directory,
                        "page": 1,
                        "per_page": 0,
                        "password": "",
                        "refresh": True,
                    }
                ).encode(),
                headers={
                    "Authorization": config["openlist"]["token"],
                    "Content-Type": "application/json",
                },
            )
            with urllib.request.urlopen(req, timeout=30) as response:
                listing = json.load(response)
            print(
                json.dumps(
                    {
                        "directory": directory,
                        "code": listing.get("code"),
                        "files": [
                            {"name": item["name"], "size": item.get("size")}
                            for item in listing.get("data", {}).get("content") or []
                        ],
                    },
                    ensure_ascii=False,
                )
            )
        for kind in ["offline_download", "offline_download_transfer"]:
            for completed in ["undone", "done"]:
                req = urllib.request.Request(
                    config["openlist"]["url"].rstrip("/")
                    + "/api/task/"
                    + kind
                    + "/"
                    + completed,
                    headers={"Authorization": config["openlist"]["token"]},
                )
                try:
                    with urllib.request.urlopen(req, timeout=30) as response:
                        tasks = json.load(response).get("data", [])
                    for task in tasks or []:
                        if task.get("id") == remote.get(
                            "task_id"
                        ) or "Rust Acceptance" in task.get("name", ""):
                            print(
                                json.dumps(
                                    {
                                        "queue": kind + "/" + completed,
                                        "name": task.get("name"),
                                        "state": task.get("state"),
                                        "status": task.get("status"),
                                        "progress": task.get("progress"),
                                    },
                                    ensure_ascii=False,
                                )
                            )
                except urllib.error.HTTPError as error:
                    print(json.dumps({"queue": kind, "status": error.code}))


def final_snapshot():
    baseline = Path((ROOT / "backups/rust-rollback-path").read_text().strip())
    destination = baseline / "cutover-data"
    destination.mkdir(mode=0o700, exist_ok=True)
    with sqlite3.connect(ROOT / "data/task_mementos.db") as connection:
        active = connection.execute(
            "SELECT COUNT(*) FROM task_mementos WHERE state NOT IN ('failed', 'completed', 'cancelled')"
        ).fetchone()[0]
    if active:
        raise RuntimeError(
            "legacy downloads are still active; refusing to switch workers"
        )
    for name in ["data.db", "task_mementos.db"]:
        with (
            sqlite3.connect(ROOT / "data" / name) as source,
            sqlite3.connect(destination / name) as target,
        ):
            source.backup(target)
            if target.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
                raise RuntimeError("database integrity failed before cutover")
        os.chmod(destination / name, 0o600)
    shutil.copy2(ROOT / "config.toml", baseline / "cutover-config.toml")
    print("Final stopped-service SQLite snapshots verified; no active legacy downloads")


def restart_check():
    cookie = session_cookie()
    before = subprocess.check_output(
        ["systemctl", "show", "openlist-ani.service", "-p", "MainPID", "--value"],
        text=True,
    ).strip()
    status, result = request("http://127.0.0.1:26666", "/api/restart", cookie, {})
    assert status == 200 and result.get("success")
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        after = subprocess.check_output(
            ["systemctl", "show", "openlist-ani.service", "-p", "MainPID", "--value"],
            text=True,
        ).strip()
        if after not in ("0", before):
            try:
                status, _ = request("http://127.0.0.1:26666", "/readyz", cookie)
                if status == 200:
                    print(
                        json.dumps(
                            {
                                "restart": "passed",
                                "before_pid": before,
                                "after_pid": after,
                            }
                        )
                    )
                    return
            except urllib.error.URLError:
                pass
        time.sleep(1)
    raise RuntimeError("service did not restart into a new healthy process")


def production():
    cookie = session_cookie()
    status, state = request("http://127.0.0.1:26666", "/api/ui/state", cookie)
    configured = tomllib.loads((ROOT / "config.toml").read_text())["rss"][
        "subscriptions"
    ]
    assert status == 200 and len(state["rss_subscriptions"]) == len(configured)
    report = {
        "subscriptions": len(state["rss_subscriptions"]),
        "active_feeds": len(state["rss_urls"]),
        "rss_status": state["rss_status"],
        "tasks": [
            {
                "title": task["title"],
                "state": task["state"],
                "error": task.get("error_message"),
            }
            for task in state["tasks"]
        ],
        "config_sha256": sha256(ROOT / "config.toml"),
    }
    report["integrity"] = {}
    for name in ["data.db", "task_mementos.db"]:
        with sqlite3.connect(
            f'file:{ROOT / "data" / name}?mode=ro', uri=True
        ) as connection:
            report["integrity"][name] = connection.execute(
                "PRAGMA integrity_check"
            ).fetchone()[0]
    timings = []
    for _ in range(30):
        start = time.monotonic()
        code, _ = request("http://127.0.0.1:26666", "/api/ui/state", cookie)
        assert code == 200
        timings.append((time.monotonic() - start) * 1000)
    report["state_p95_ms"] = round(sorted(timings)[28], 2)
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "mode",
        choices=[
            "backup",
            "inspect",
            "prepare",
            "probe",
            "external",
            "download",
            "resume-download",
            "download-status",
            "final-snapshot",
            "production",
            "restart-check",
        ],
    )
    args = parser.parse_args()
    {
        "backup": backup,
        "inspect": inspect,
        "prepare": prepare,
        "probe": probe,
        "external": external,
        "download": download,
        "resume-download": lambda: download(True),
        "download-status": download_status,
        "final-snapshot": final_snapshot,
        "production": production,
        "restart-check": restart_check,
    }[args.mode]()
