"""HTTP service facade for API schema conversion."""

from __future__ import annotations

from openlist_ani.application.anime_library_ingestion.application_service import (
    AnimeLibraryApplicationService,
)
from openlist_ani.domain.anime_release import AnimeRelease
from openlist_ani.domain.download_task.memento import TaskMemento
from openlist_ani.application.anime_library_ingestion.exclusions import (
    normalize_exclude_patterns,
)
from openlist_ani.adapters.outbound.configuration import config
from openlist_ani.assistant.skill_support.mikan_client import MikanClient

from .schema import (
    DownloadTaskResponse,
    ParseRSSEntry,
    ParseRSSResponse,
    ResolveMagnetFile,
    ResolveMagnetResponse,
    ResolveTorrentResponse,
)


def _build_task_response(task: TaskMemento) -> DownloadTaskResponse:
    release = task.release
    downloader_payload = (
        task.downloader.payload
        if task.downloader is not None
        else {}
    )
    return DownloadTaskResponse(
        id=task.task_id,
        title=release.title,
        download_url=release.download_url,
        state=task.state.value,
        anime_name=release.anime_name,
        season=release.season,
        episode=release.episode,
        fansub=release.fansub,
        quality=release.quality.value if release.quality else None,
        progress=downloader_payload.get("progress"),
        error_message=task.retry.last_error,
        retry_count=task.retry.retry_count,
        created_at=task.created_at,
        updated_at=task.updated_at,
        started_at=task.started_at,
        completed_at=task.completed_at,
        save_path=task.base_path,
        final_path=task.output_path,
    )


def _build_parse_entry(index: int, release: AnimeRelease) -> ParseRSSEntry:
    return ParseRSSEntry(
        index=index,
        title=release.title,
        download_url=release.download_url,
        anime_name=release.anime_name,
        episode=release.episode,
        fansub=release.fansub,
        quality=release.quality.value if release.quality else None,
        languages=[lang.value for lang in (release.languages or [])],
    )


def _build_magnet_response(result) -> ResolveMagnetResponse:
    return ResolveMagnetResponse(
        success=result.success,
        message=result.message,
        title=result.title,
        source=result.source,
        file_count=result.file_count,
        files=[ResolveMagnetFile(name=f.name, size=f.size) for f in result.files],
    )


class BackendApiService:
    """Singleton bridge between FastAPI routes and application use cases."""

    _instance: BackendApiService | None = None

    def __init__(self, application_service: AnimeLibraryApplicationService) -> None:
        self._application_service = application_service

    @classmethod
    def init(
        cls, application_service: AnimeLibraryApplicationService
    ) -> BackendApiService:
        cls._instance = cls(application_service)
        return cls._instance

    @classmethod
    def get(cls) -> BackendApiService:
        if cls._instance is None:
            raise RuntimeError("BackendApiService not initialized")
        return cls._instance

    @property
    def pipeline(self):
        return self._application_service.pipeline

    def add_rss_url(self, url: str) -> tuple[bool, str, list[str]]:
        return self._application_service.add_rss_url(url)

    async def resolve_rss_subscription(
        self, url: str, preferred_name: str = ""
    ) -> dict[str, object]:
        return await self._application_service.resolve_rss_subscription(
            url, preferred_name
        )

    async def refresh_rss_subscription(
        self,
        url: str,
        *,
        preferred_name: str = "",
        preferred_tmdb_id: int | None = None,
    ) -> dict[str, object]:
        return await self._application_service.refresh_rss_subscription(
            url,
            preferred_name=preferred_name,
            preferred_tmdb_id=preferred_tmdb_id,
        )

    def add_rss_subscription(
        self,
        url: str,
        *,
        name: str = "",
        anime_name: str = "",
        download_directory_name: str = "",
        tmdb_id: int | None = None,
        poster_url: str = "",
        season: int | None = None,
        exclude_patterns: list[str] | None = None,
    ) -> tuple[bool, str, list[str]]:
        return self._application_service.add_rss_url(
            url,
            name=name,
            anime_name=anime_name,
            download_directory_name=download_directory_name,
            tmdb_id=tmdb_id,
            poster_url=poster_url,
            season=season,
            exclude_patterns=exclude_patterns,
        )

    def list_rss_subscriptions(self) -> list[dict[str, object]]:
        return self._application_service.list_rss_subscriptions()

    def update_rss_subscription(
        self,
        url: str,
        *,
        name: str | None = None,
        anime_name: str | None = None,
        download_directory_name: str | None = None,
        enabled: bool | None = None,
        tmdb_id: int | None = None,
        poster_url: str | None = None,
        season: int | None = None,
        exclude_patterns: list[str] | None = None,
    ) -> tuple[bool, str, list[str]]:
        return self._application_service.update_rss_subscription(
            url,
            name=name,
            anime_name=anime_name,
            download_directory_name=download_directory_name,
            enabled=enabled,
            tmdb_id=tmdb_id,
            poster_url=poster_url,
            season=season,
            exclude_patterns=exclude_patterns,
        )

    def correct_rss_subscription(
        self,
        original_url: str,
        *,
        url: str,
        name: str = "",
        anime_name: str = "",
        download_directory_name: str = "",
        tmdb_id: int | None = None,
        poster_url: str = "",
        season: int | None = None,
        exclude_patterns: str | list[str] = "",
    ) -> tuple[bool, str, list[str]]:
        return self._application_service.correct_rss_subscription(
            original_url,
            url=url,
            name=name,
            anime_name=anime_name,
            download_directory_name=download_directory_name,
            tmdb_id=tmdb_id,
            poster_url=poster_url,
            season=season,
            exclude_patterns=normalize_exclude_patterns(exclude_patterns),
        )

    async def preview_rss_subscription(
        self,
        url: str,
        *,
        preferred_name: str = "",
        preferred_anime_name: str = "",
        preferred_download_directory_name: str = "",
        exclude_patterns: str | list[str] = "",
    ) -> dict[str, object]:
        return await self._application_service.preview_rss_subscription(
            url,
            preferred_name=preferred_name,
            preferred_anime_name=preferred_anime_name,
            preferred_download_directory_name=preferred_download_directory_name,
            exclude_patterns=normalize_exclude_patterns(exclude_patterns),
        )

    def update_global_exclude_patterns(
        self, exclude_patterns: str | list[str]
    ) -> dict[str, object]:
        success, message = self._application_service.update_global_exclude_patterns(
            normalize_exclude_patterns(exclude_patterns)
        )
        return {
            "success": success,
            "message": message,
            "exclude_patterns": self._application_service.get_global_exclude_patterns(),
        }

    def global_exclude_patterns(self) -> list[str]:
        return self._application_service.get_global_exclude_patterns()

    async def validate_download_path(self, path: str) -> tuple[bool, str]:
        return await self._application_service.validate_download_path(path)

    async def validate_openlist_url(self, url: str) -> tuple[bool, str]:
        return await self._application_service.validate_openlist_url(url)

    async def validate_download_path_at_url(
        self, url: str, path: str
    ) -> tuple[bool, str]:
        return await self._application_service.validate_download_path_at_url(url, path)

    def validate_rename_format(self, rename_format: str) -> tuple[bool, str]:
        return self._application_service.validate_rename_format(rename_format)

    def update_runtime_openlist_settings(
        self, *, download_path: str, rename_format: str
    ) -> None:
        self._application_service.update_runtime_openlist_settings(
            download_path=download_path,
            rename_format=rename_format,
        )

    def update_runtime_openlist_url(self, url: str) -> None:
        self._application_service.update_runtime_openlist_url(url)

    def update_runtime_rss_interval(self, interval_seconds: int) -> None:
        self._application_service.update_runtime_rss_interval(interval_seconds)

    def update_runtime_max_download_retries(self, max_retries: int) -> None:
        self._application_service.update_runtime_max_download_retries(max_retries)

    def remove_rss_url(self, url: str) -> tuple[bool, str, list[str]]:
        return self._application_service.remove_rss_url(url)

    async def scan_rss_now(self) -> dict[str, object]:
        return await self._application_service.scan_rss_now()

    def schedule_rss_scan_for_url(self, source_url: str) -> bool:
        """Start a background targeted scan for a newly saved RSS source."""
        return self._application_service.schedule_rss_scan_for_url(source_url)

    async def search_mikan(
        self,
        keyword: str,
        *,
        base_url: str | None = None,
    ) -> dict[str, object]:
        """Search the configured Mikan-compatible site for bangumi."""
        client = MikanClient(
            username=config.mikan.username,
            password=config.mikan.password,
            base_url=base_url or config.mikan.base_url,
        )
        try:
            results = await client.search_bangumi(keyword.strip())
            return {
                "success": True,
                "results": results,
                "base_url": base_url or config.mikan.base_url,
            }
        except Exception as exc:
            return {"success": False, "message": f"Mikan 搜索失败：{exc}"}
        finally:
            await client.close()

    async def list_mikan_groups(
        self,
        bangumi_id: int,
        *,
        base_url: str | None = None,
    ) -> dict[str, object]:
        """List subtitle groups and ready-to-use RSS URLs for a bangumi."""
        client = MikanClient(
            username=config.mikan.username,
            password=config.mikan.password,
            base_url=base_url or config.mikan.base_url,
        )
        try:
            groups = await client.fetch_bangumi_subgroups(bangumi_id)
            return {
                "success": True,
                "bangumi_id": bangumi_id,
                "all_rss_url": client.rss_url(bangumi_id),
                "groups": [
                    {
                        "id": group.get("id"),
                        "name": group.get("name", "未命名字幕组"),
                        "release_count": len(group.get("releases", [])),
                        "rss_url": client.rss_url(bangumi_id, group.get("id")),
                    }
                    for group in groups
                ],
            }
        except Exception as exc:
            return {"success": False, "message": f"Mikan 字幕组读取失败：{exc}"}
        finally:
            await client.close()

    def mikan_rss_url(
        self,
        bangumi_id: int,
        subgroup_id: int | None = None,
        *,
        base_url: str | None = None,
    ) -> dict[str, object]:
        """Build the selected Mikan RSS URL using the current site setting."""
        client = MikanClient(
            username=config.mikan.username,
            password=config.mikan.password,
            base_url=base_url or config.mikan.base_url,
        )
        return {
            "success": True,
            "rss_url": client.rss_url(bangumi_id, subgroup_id),
        }

    def rss_status(self) -> dict[str, object]:
        return self._application_service.rss_status()

    async def create_download(
        self,
        download_url: str,
        title: str,
    ) -> tuple[bool, str, DownloadTaskResponse | None]:
        outcome = await self._application_service.create_download(download_url, title)
        return (
            outcome.success,
            outcome.message,
            _build_task_response(outcome.task) if outcome.task else None,
        )

    def list_downloads(self) -> list[DownloadTaskResponse]:
        return [
            _build_task_response(task)
            for task in self._application_service.list_downloads()
        ]

    def get_download(self, task_id: str) -> DownloadTaskResponse | None:
        task = self._application_service.get_download(task_id)
        return _build_task_response(task) if task else None

    async def parse_rss(
        self,
        url: str,
        limit: int | None = None,
    ) -> ParseRSSResponse:
        outcome = await self._application_service.parse_rss(url, limit)
        entries = outcome.entries or []
        return ParseRSSResponse(
            success=outcome.success,
            message=outcome.message,
            total=outcome.total,
            entries=[
                _build_parse_entry(index, release)
                for index, release in enumerate(entries)
            ],
        )

    async def resolve_magnet(
        self,
        magnet: str,
        metadata_timeout: int = 30,
    ) -> ResolveMagnetResponse:
        result = await self._application_service.resolve_magnet(
            magnet, metadata_timeout=metadata_timeout
        )
        return _build_magnet_response(result)

    async def resolve_torrent(self, url: str) -> ResolveTorrentResponse:
        result = await self._application_service.resolve_torrent(url)
        magnet_response = _build_magnet_response(result)
        return ResolveTorrentResponse(**magnet_response.model_dump())
