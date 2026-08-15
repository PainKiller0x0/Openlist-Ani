#!/usr/bin/env python3
"""Keep the rotating Xunlei token shared by OpenList and SmartStrm.

The script intentionally never logs access/refresh tokens.  OpenList is used
as the persistent storage owner; if SmartStrm refreshes first, the API update
path is used and the final token returned by OpenList is copied back.
"""

from __future__ import annotations

import argparse
import base64
import fcntl
import hashlib
import json
import logging
import os
import pathlib
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

import yaml


SMART_CONFIG = pathlib.Path(
    os.getenv("SMARTSTRM_CONFIG", "/opt/smartstrm/config/config.yaml")
)
OPENLIST_DB = pathlib.Path(
    os.getenv("OPENLIST_DB", "/opt/openlist/data/data.db")
)
OPENLIST_URL = os.getenv("OPENLIST_URL", "http://127.0.0.1:5244").rstrip("/")
OPENLIST_STORAGE_ID = int(os.getenv("OPENLIST_STORAGE_ID", "2"))
OPENLIST_TOKEN_FILE = pathlib.Path(
    os.getenv("OPENLIST_TOKEN_FILE", "/etc/xunlei-token-bridge.env")
)
LOCK_FILE = pathlib.Path(
    os.getenv("XUNLEI_TOKEN_BRIDGE_LOCK", "/run/xunlei-token-bridge.lock")
)


class SmartStrmBusy(RuntimeError):
    pass


def token_hash(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()[:12] if value else "-"


def jwt_exp(value: str) -> int:
    try:
        part = value.split(".")[1]
        part += "=" * ((4 - len(part) % 4) % 4)
        return int(json.loads(base64.urlsafe_b64decode(part)).get("exp", 0))
    except (IndexError, ValueError, TypeError, json.JSONDecodeError):
        return 0


def load_openlist_admin_token() -> str:
    for line in OPENLIST_TOKEN_FILE.read_text(encoding="utf-8").splitlines():
        if line.startswith("OPENLIST_ADMIN_TOKEN="):
            return line.split("=", 1)[1].strip()
    raise RuntimeError("OPENLIST_ADMIN_TOKEN is missing")


def read_smartstrm() -> tuple[dict, dict]:
    document = yaml.safe_load(SMART_CONFIG.read_text(encoding="utf-8")) or {}
    storage = next(
        item for item in document.get("storages", []) if item.get("driver") == "xunlei"
    )
    return document, storage


def read_openlist_db() -> dict:
    with sqlite3.connect(OPENLIST_DB) as connection:
        row = connection.execute(
            "select addition from x_storages where id = ?", (OPENLIST_STORAGE_ID,)
        ).fetchone()
    if not row:
        raise RuntimeError(f"OpenList storage {OPENLIST_STORAGE_ID} was not found")
    addition = row[0]
    return json.loads(addition) if isinstance(addition, str) else addition


def api_json(method: str, path: str, body: dict | None = None) -> dict:
    token = load_openlist_admin_token()
    encoded = None if body is None else json.dumps(body, ensure_ascii=False).encode()
    request = urllib.request.Request(
        OPENLIST_URL + path,
        data=encoded,
        method=method,
        headers={
            "Authorization": token,
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            result = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")[:300]
        raise RuntimeError(f"OpenList API HTTP {exc.code}: {detail}") from exc
    if result.get("code") not in (None, 200):
        raise RuntimeError(f"OpenList API code {result.get('code')}: {result.get('message')}")
    return result


def openlist_storage() -> dict:
    result = api_json("GET", f"/api/admin/storage/get?id={OPENLIST_STORAGE_ID}")
    data = result.get("data") or {}
    addition = data.get("addition")
    if isinstance(addition, str):
        data = dict(data)
        data["addition"] = json.loads(addition)
    return data


def openlist_tokens(data: dict) -> tuple[str, str]:
    addition = data.get("addition") or {}
    return addition.get("access_token", ""), addition.get("refresh_token", "")


def atomically_write_smartstrm(document: dict, access: str, refresh: str) -> None:
    for item in document.get("storages", []):
        if item.get("driver") == "xunlei":
            item["access_token"] = access
            item["refresh_token"] = refresh
    directory = SMART_CONFIG.parent
    with tempfile.NamedTemporaryFile(
        "w", encoding="utf-8", dir=directory, prefix=".config.yaml.", delete=False
    ) as handle:
        yaml.safe_dump(document, handle, allow_unicode=True, sort_keys=False)
        temporary = pathlib.Path(handle.name)
    os.chmod(temporary, SMART_CONFIG.stat().st_mode & 0o777)
    os.replace(temporary, SMART_CONFIG)


def smartstrm_busy() -> bool:
    """Return true when a client still has a live SmartStrm connection."""
    try:
        result = subprocess.run(
            ["ss", "-Htn", "state", "established"],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.SubprocessError):
        # Fail closed: a token refresh can wait for the next check, but a
        # false negative here could interrupt an active stream.
        return True
    for line in result.stdout.splitlines():
        fields = line.split()
        if len(fields) >= 5 and re.search(r":(?:8024|8025)$", fields[3]):
            return True
    return False


def apply_smartstrm_tokens(document: dict, access: str, refresh: str) -> None:
    if smartstrm_busy():
        raise SmartStrmBusy("active SmartStrm connection")
    atomically_write_smartstrm(document, access, refresh)
    subprocess.run(
        ["podman", "restart", "smartstrm-vps"],
        check=True,
        timeout=45,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )


def update_openlist_tokens(data: dict, access: str, refresh: str) -> dict:
    updated = dict(data)
    addition = dict(updated.get("addition") or {})
    addition["access_token"] = access
    addition["refresh_token"] = refresh
    updated["addition"] = json.dumps(addition, ensure_ascii=False, separators=(",", ":"))
    api_json("POST", "/api/admin/storage/update", updated)
    # OpenList may rotate the refresh token while applying the update.  Always
    # read back the resulting storage instead of assuming the submitted token
    # is still authoritative.
    return openlist_storage()


def state_label(access: str, refresh: str) -> str:
    return f"access_exp={jwt_exp(access)} refresh={token_hash(refresh)}"


def sync(check_only: bool = False) -> int:
    smart_document, smart = read_smartstrm()
    smart_access = smart.get("access_token", "")
    smart_refresh = smart.get("refresh_token", "")
    # The API read is authoritative for OpenList's current state.  The local
    # database is not written by this bridge, avoiding SQLite races with the
    # OpenList process.
    open_data = openlist_storage()
    open_access, open_refresh = openlist_tokens(open_data)

    if smart_refresh == open_refresh and smart_access == open_access:
        logging.info("tokens in sync (%s)", state_label(smart_access, smart_refresh))
        return 0

    now = int(time.time())
    smart_exp = jwt_exp(smart_access)
    open_exp = jwt_exp(open_access)
    if open_exp > smart_exp or (open_exp == smart_exp and open_refresh):
        source = "openlist"
    elif smart_exp > open_exp:
        source = "smartstrm"
    else:
        # Both access tokens are unusable/opaque.  Prefer the persisted
        # OpenList state; it is the account used by OpenList-Ani.
        source = "openlist"

    if check_only:
        logging.warning(
            "token mismatch; source=%s smart=(%s) openlist=(%s) now=%s",
            source,
            state_label(smart_access, smart_refresh),
            state_label(open_access, open_refresh),
            now,
        )
        return 2

    if source == "openlist":
        apply_smartstrm_tokens(smart_document, open_access, open_refresh)
        logging.info("copied OpenList token to SmartStrm and restarted SmartStrm")
        return 0

    # SmartStrm refreshed first.  Let OpenList persist it, then copy back the
    # final token because the update operation itself may rotate it.
    final_open = update_openlist_tokens(open_data, smart_access, smart_refresh)
    final_access, final_refresh = openlist_tokens(final_open)
    if final_access != smart_access or final_refresh != smart_refresh:
        apply_smartstrm_tokens(smart_document, final_access, final_refresh)
        logging.info("OpenList rotated the token during sync; copied final token back to SmartStrm")
    else:
        logging.info("persisted SmartStrm token in OpenList")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    LOCK_FILE.parent.mkdir(parents=True, exist_ok=True)
    with LOCK_FILE.open("w") as lock:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            logging.info("another bridge run is active")
            return 0
        try:
            return sync(args.check_only)
        except SmartStrmBusy:
            logging.info("active playback detected; token update deferred until the next idle check")
            return 0
        except Exception:
            logging.exception("token bridge failed")
            return 1


if __name__ == "__main__":
    raise SystemExit(main())
