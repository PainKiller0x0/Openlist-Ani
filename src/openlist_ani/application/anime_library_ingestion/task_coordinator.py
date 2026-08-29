from __future__ import annotations

import asyncio
import uuid
from collections import OrderedDict

from openlist_ani.application.anime_library_ingestion.ports import (
    EventPublisherPort,
    TaskMementoStorePort,
)
from openlist_ani.application.common import OAniEvent, OAniEventType
from openlist_ani.domain.anime_release import AnimeRelease
from openlist_ani.domain.download_task.memento import RetryMemento, TaskMemento
from openlist_ani.domain.download_task.task import DownloadState, TERMINAL_STATES


class TaskCoordinator:
    """Runtime task registry backed by the durable memento store."""

    _TERMINAL_STATES = TERMINAL_STATES

    def __init__(
        self,
        task_store: TaskMementoStorePort,
        event_publisher: EventPublisherPort,
        default_base_path: str,
        max_retries: int = 3,
        terminal_history_limit: int = 100,
    ) -> None:
        self._task_store = task_store
        self._event_publisher = event_publisher
        self._default_base_path = default_base_path
        self._max_retries = max(0, int(max_retries))
        self._tasks: dict[str, TaskMemento] = {}
        self._terminal_history: OrderedDict[str, TaskMemento] = OrderedDict()
        self._terminal_history_limit = terminal_history_limit
        self._reservation_lock = asyncio.Lock()

    def load_all(self) -> list[TaskMemento]:
        tasks = self._task_store.load_all()
        self._tasks = {}
        self._terminal_history.clear()
        for task in tasks:
            if task.state in self._TERMINAL_STATES:
                self._remember_terminal(task)
            else:
                self._tasks[task.task_id] = task
        return list(tasks)

    def save(self, task_memento: TaskMemento) -> None:
        self._task_store.save(task_memento)
        if task_memento.state in self._TERMINAL_STATES:
            self._tasks.pop(task_memento.task_id, None)
            self._remember_terminal(task_memento)
            return
        self._terminal_history.pop(task_memento.task_id, None)
        self._tasks[task_memento.task_id] = task_memento

    def delete(self, task_id: str) -> None:
        self._tasks.pop(task_id, None)
        self._task_store.delete(task_id)

    def atomic_flush(self) -> None:
        self._task_store.atomic_flush()

    def update_default_base_path(self, base_path: str) -> None:
        """Use a new destination for tasks reserved after this point."""
        self._default_base_path = base_path

    def update_max_retries(self, max_retries: int) -> None:
        """Apply the retry limit to newly created and active tasks."""
        self._max_retries = max(0, int(max_retries))
        for task in self.list_tasks():
            if task.state in self._TERMINAL_STATES:
                continue
            task.retry.max_retries = self._max_retries
            task.touch()
            self.save(task)

    def cancel_tasks_for_source(
        self,
        source_url: str,
        *,
        anime_names: set[str] | None = None,
        reason: str = "RSS subscription stopped",
    ) -> int:
        """Cancel queued and failed tasks belonging to an RSS subscription."""
        names = {name.strip() for name in (anime_names or set()) if name.strip()}
        cancelled = 0
        for task in self.list_tasks():
            if task.state == DownloadState.COMPLETED:
                continue
            same_source = task.release.source_url == source_url
            legacy_name = (
                task.release.source_url is None
                and task.release.anime_name in names
            )
            if not (same_source or legacy_name):
                continue
            task.state = DownloadState.CANCELLED
            task.retry.last_error = reason
            task.touch()
            self.save(task)
            cancelled += 1
        return cancelled

    async def reserve_download_task(
        self,
        release: AnimeRelease,
        base_path: str | None = None,
    ) -> TaskMemento | None:
        async with self._reservation_lock:
            if self.is_downloading(release):
                return None

            task = TaskMemento(
                task_id=str(uuid.uuid4()),
                state=DownloadState.PENDING,
                release=release,
                base_path=base_path or self._default_base_path,
                retry=RetryMemento(max_retries=self._max_retries),
            )
            self.save(task)
            await self._event_publisher.publish(
                OAniEvent(OAniEventType.TASK_CREATED, {"task_id": task.task_id})
            )
            return task

    async def prepare_manual_retry(
        self, task_id: str
    ) -> tuple[TaskMemento | None, str | None]:
        """Reset one failed task for a fresh download attempt."""
        async with self._reservation_lock:
            task = self.get_task(task_id)
            if task is None:
                return None, "Task not found"
            if task.state != DownloadState.FAILED:
                return (
                    None,
                    f"Task is {task.state.value}; only failed tasks can be retried",
                )

            duplicate = any(
                other.task_id != task.task_id
                and other.release.download_url == task.release.download_url
                and other.state
                not in {
                    DownloadState.COMPLETED,
                    DownloadState.FAILED,
                    DownloadState.CANCELLED,
                }
                for other in self.list_tasks()
            )
            if duplicate:
                return None, "The same release is already queued or downloading"

            # The old OpenList task may have been deleted. Clear every
            # provider checkpoint so the downloader submits a fresh task.
            task.state = DownloadState.PENDING
            task.downloader = None
            task.pipeline.next_buffer = "download"
            task.pipeline.downloaded_directory_path = None
            task.pipeline.downloaded_filename = None
            task.pipeline.renamed_path = None
            task.output_path = None
            task.retry.retry_count = 0
            task.retry.last_error = None
            task.started_at = None
            task.completed_at = None
            task.archived = False
            task.touch()
            self.save(task)
            return task, None

    def register_task(self, task_memento: TaskMemento) -> None:
        if task_memento.state in self._TERMINAL_STATES:
            self._remember_terminal(task_memento)
            return
        self._terminal_history.pop(task_memento.task_id, None)
        self._tasks[task_memento.task_id] = task_memento

    def is_downloading(self, release: AnimeRelease) -> bool:
        return any(
            task.release.download_url == release.download_url
            and not task.archived
            and task.state not in {DownloadState.COMPLETED, DownloadState.CANCELLED}
            and (task.state != DownloadState.FAILED or release.source_url is not None)
            for task in self.list_tasks()
        )

    def list_tasks(self) -> list[TaskMemento]:
        return [*self._tasks.values(), *self._terminal_history.values()]

    def list_active_tasks(self) -> list[TaskMemento]:
        return [
            task
            for task in self._tasks.values()
            if task.state not in self._TERMINAL_STATES
        ]

    def get_task(self, task_id: str) -> TaskMemento | None:
        return self._tasks.get(task_id) or self._terminal_history.get(task_id)

    def _remember_terminal(self, task_memento: TaskMemento) -> None:
        if self._terminal_history_limit <= 0:
            return
        self._terminal_history[task_memento.task_id] = task_memento
        self._terminal_history.move_to_end(task_memento.task_id)
        while len(self._terminal_history) > self._terminal_history_limit:
            self._terminal_history.popitem(last=False)
