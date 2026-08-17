"""Small, optional SmartStrm trigger client.

OpenList-Ani does not need to know SmartStrm's webhook secret. It only posts a
successful materialized path to the local op-ops relay. The relay owns the
secret and turns the event into SmartStrm's authenticated ``a_task`` webhook.
"""

from __future__ import annotations

import asyncio
import json
import urllib.error
import urllib.request
from dataclasses import dataclass

from openlist_ani.application.common import OAniEvent
from openlist_ani.logger import logger


@dataclass(frozen=True)
class SmartStrmTriggerSettings:
    enabled: bool = False
    trigger_url: str = ""
    timeout_seconds: float = 2.0


class SmartStrmTrigger:
    """Post download-completed events without delaying the download pipeline."""

    def __init__(self, settings: SmartStrmTriggerSettings) -> None:
        self._settings = settings

    async def handle(self, event: OAniEvent) -> None:
        if not self._settings.enabled or not self._settings.trigger_url:
            return

        payload = {
            "event": "download.completed",
            "task_id": event.payload.get("task_id"),
            "path": event.payload.get("path", ""),
            "base_path": event.payload.get("base_path", ""),
        }
        try:
            status = await asyncio.to_thread(self._post, payload)
            logger.info(
                "SmartStrm trigger accepted for %s (HTTP %s)",
                payload["path"] or payload["task_id"],
                status,
            )
        except Exception as error:
            # The download is already complete. A temporarily unavailable
            # relay must not turn it into a failed pipeline task.
            logger.warning("SmartStrm trigger skipped: %s", error)

    def _post(self, payload: dict[str, object]) -> int:
        request = urllib.request.Request(
            self._settings.trigger_url,
            data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
            method="POST",
            headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "User-Agent": "openlist-ani-smartstrm-trigger/1.0",
            },
        )
        try:
            with urllib.request.urlopen(
                request, timeout=self._settings.timeout_seconds
            ) as response:
                response.read(256)
                return int(response.status)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"relay HTTP {error.code}") from error
