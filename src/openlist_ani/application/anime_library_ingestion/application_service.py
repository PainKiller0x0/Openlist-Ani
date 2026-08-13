"""HTTP-facing application service for anime library operations."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any, Protocol

from openlist_ani.application.anime_library_ingestion.pipeline import (
    AnimeLibraryIngestionPipeline,
)
from openlist_ani.application.anime_library_ingestion.ports import (
    AnimeLibraryRepositoryPort,
    MetadataParserPort,
    MetadataValidatorPort,
)
from openlist_ani.application.anime_library_ingestion.settings import (
    AnimeLibraryIngestionSettings,
)
from openlist_ani.application.anime_library_ingestion.models import ParseResult
from openlist_ani.domain.anime_release import AnimeRelease
from openlist_ani.domain.download_task.memento import TaskMemento
from openlist_ani.domain.download_task.task import DownloadState
from openlist_ani.logger import logger


class ReleaseFeedSourcePort(Protocol):
    async def fetch_feed(self, url: str) -> list[AnimeRelease]: ...


class ReleaseFeedSourceFactoryPort(Protocol):
    def create(self, url: str) -> ReleaseFeedSourcePort: ...


ResolveMagnet = Callable[..., Awaitable[Any]]
ResolveTorrent = Callable[..., Awaitable[Any]]
LookupTMDBShow = Callable[[str], Awaitable[dict[str, Any] | None]]


@dataclass(frozen=True)
class CreateDownloadOutcome:
    success: bool
    message: str
    task: TaskMemento | None = None


@dataclass(frozen=True)
class ParseRSSOutcome:
    success: bool
    message: str
    total: int = 0
    entries: list[AnimeRelease] | None = None


class AnimeLibraryApplicationService:
    """Application facade for HTTP-facing anime library operations."""

    def __init__(
        self,
        pipeline: AnimeLibraryIngestionPipeline,
        metadata_parser: MetadataParserPort,
        metadata_validator: MetadataValidatorPort,
        anime_library_repository: AnimeLibraryRepositoryPort,
        settings: AnimeLibraryIngestionSettings,
        feed_factory: ReleaseFeedSourceFactoryPort,
        resolve_magnet_func: ResolveMagnet,
        resolve_torrent_func: ResolveTorrent,
        get_rss_urls: Callable[[], list[str]],
        add_rss_url_func: Callable[..., None],
        remove_rss_url_func: Callable[[str], bool],
        get_rss_subscriptions: Callable[[], list[dict[str, Any]]] | None = None,
        update_rss_subscription_func: Callable[..., bool] | None = None,
        lookup_tmdb_show: LookupTMDBShow | None = None,
    ) -> None:
        self._pipeline = pipeline
        self._metadata_parser = metadata_parser
        self._metadata_validator = metadata_validator
        self._anime_library_repository = anime_library_repository
        self._settings = settings
        self._feed_factory = feed_factory
        self._resolve_magnet = resolve_magnet_func
        self._resolve_torrent = resolve_torrent_func
        self._get_rss_urls = get_rss_urls
        self._get_rss_subscriptions = get_rss_subscriptions or (
            lambda: [{"url": url, "name": "", "enabled": True} for url in get_rss_urls()]
        )
        self._add_rss_url = add_rss_url_func
        self._remove_rss_url = remove_rss_url_func
        self._update_rss_subscription = update_rss_subscription_func or (
            lambda _url, **_kwargs: False
        )
        self._lookup_tmdb_show = lookup_tmdb_show

    @property
    def pipeline(self) -> AnimeLibraryIngestionPipeline:
        return self._pipeline

    def add_rss_url(
        self,
        url: str,
        *,
        name: str = "",
        tmdb_id: int | None = None,
        poster_url: str = "",
    ) -> tuple[bool, str, list[str]]:
        current_urls = self._get_rss_urls()
        if url in current_urls:
            return False, f"URL already exists: {url}", current_urls

        try:
            self._add_rss_url(
                url,
                name=name,
                tmdb_id=tmdb_id,
                poster_url=poster_url,
            )
        except TypeError:
            # Keep old adapters and test doubles working while the richer
            # subscription metadata API rolls out.
            self._add_rss_url(url)
        updated_urls = self._get_rss_urls()
        feed_reader = self._pipeline.feed_reader
        set_urls = getattr(feed_reader, "set_urls", None)
        if set_urls is not None:
            set_urls(updated_urls)
        logger.info(f"Added RSS URL: {url}")
        return True, f"RSS URL added successfully: {url}", updated_urls

    def list_rss_subscriptions(self) -> list[dict[str, Any]]:
        return self._get_rss_subscriptions()

    async def resolve_rss_subscription(
        self, url: str, preferred_name: str = ""
    ) -> dict[str, Any]:
        """Infer a display name and optional TMDB poster from the feed."""
        parsed = await self.parse_rss(url, limit=5)
        if not parsed.success:
            return {"name": preferred_name.strip(), "error": parsed.message}

        inferred_name = preferred_name.strip()
        tmdb_id: int | None = None
        for release in parsed.entries or []:
            if not inferred_name and release.anime_name:
                inferred_name = release.anime_name.strip()
            try:
                parse_results = await self._metadata_parser.parse([release])
                validated = await self._metadata_validator.validate(parse_results)
                parsed_result = validated[0] if validated else None
                result = parsed_result.result if parsed_result else None
                if parsed_result and parsed_result.success and result is not None:
                    inferred_name = inferred_name or result.anime_name.strip()
                    tmdb_id = result.tmdb_id
                    if result.anime_name.strip() and not preferred_name.strip():
                        inferred_name = result.anime_name.strip()
                    if tmdb_id is not None:
                        break
            except Exception as e:
                logger.debug(f"Subscription metadata inference failed: {e}")

        tmdb = None
        if self._lookup_tmdb_show and inferred_name:
            try:
                tmdb = await self._lookup_tmdb_show(inferred_name)
            except Exception as e:
                logger.debug(f"TMDB poster lookup failed for {inferred_name}: {e}")
        if tmdb:
            tmdb_id = tmdb.get("id") or tmdb_id
            poster_url = tmdb.get("poster_url") or ""
        else:
            poster_url = ""

        return {
            "name": inferred_name,
            "tmdb_id": tmdb_id,
            "poster_url": poster_url,
        }

    def update_rss_subscription(
        self,
        url: str,
        *,
        name: str | None = None,
        enabled: bool | None = None,
    ) -> tuple[bool, str, list[str]]:
        if not self._update_rss_subscription(url, name=name, enabled=enabled):
            return False, f"RSS URL not found: {url}", self._get_rss_urls()
        updated_urls = self._get_rss_urls()
        feed_reader = self._pipeline.feed_reader
        set_urls = getattr(feed_reader, "set_urls", None)
        if set_urls is not None:
            set_urls(updated_urls)
        state = "resumed" if enabled else "paused" if enabled is False else "updated"
        return True, f"RSS subscription {state}: {url}", updated_urls

    async def create_download(
        self,
        download_url: str,
        title: str,
    ) -> CreateDownloadOutcome:
        if await self._anime_library_repository.is_downloaded(title):
            logger.info(f"Release already downloaded, skipping: {title}")
            return CreateDownloadOutcome(False, f"Already downloaded: {title}")

        release = AnimeRelease(title=title, download_url=download_url)
        if self._pipeline.task_coordinator.is_downloading(release):
            return CreateDownloadOutcome(False, f"Already downloading: {title}")

        await self._enrich_release(release)
        task = await self._pipeline.submit_download(
            release, self._settings.download_path
        )
        if task is None:
            return CreateDownloadOutcome(False, f"Already downloading: {title}")

        logger.debug(f"Download task created: {title} (id={task.task_id})")
        return CreateDownloadOutcome(True, f"Download started: {title}", task)

    def list_downloads(self) -> list[TaskMemento]:
        return [
            task
            for task in self._pipeline.task_coordinator.list_tasks()
            if task.state
            not in {
                DownloadState.COMPLETED,
                DownloadState.CANCELLED,
            }
        ]

    def get_download(self, task_id: str) -> TaskMemento | None:
        return self._pipeline.task_coordinator.get_task(task_id)

    async def scan_rss_now(self) -> dict[str, object]:
        return await self._pipeline.scan_rss_now()

    def rss_status(self) -> dict[str, object]:
        return self._pipeline.rss_status()

    def remove_rss_url(self, url: str) -> tuple[bool, str, list[str]]:
        current_urls = self._get_rss_urls()
        known_subscriptions = self._get_rss_subscriptions()
        known = url in current_urls or any(
            str(item.get("url", "")) == url for item in known_subscriptions
        )
        if not known:
            return False, f"RSS URL not found: {url}", current_urls
        if not self._remove_rss_url(url):
            return False, f"RSS URL not found: {url}", current_urls
        updated_urls = self._get_rss_urls()
        feed_reader = self._pipeline.feed_reader
        set_urls = getattr(feed_reader, "set_urls", None)
        if set_urls is not None:
            set_urls(updated_urls)
        return True, f"RSS URL removed: {url}", updated_urls

    async def parse_rss(
        self,
        url: str,
        limit: int | None = None,
    ) -> ParseRSSOutcome:
        if not url:
            return ParseRSSOutcome(success=False, message="'url' is required.")

        try:
            feed_source = self._feed_factory.create(url)
        except ValueError as e:
            return ParseRSSOutcome(
                success=False, message=f"Cannot pick feed source for URL: {e}"
            )

        try:
            entries = await feed_source.fetch_feed(url)
        except Exception as e:
            logger.warning(f"parse_rss: feed fetch failed for {url}: {e}")
            return ParseRSSOutcome(success=False, message=f"Failed to fetch RSS: {e}")

        total = len(entries)
        if limit is not None and limit > 0:
            entries = entries[:limit]

        message = (
            f"Parsed {len(entries)} of {total} entries"
            if limit and total > len(entries)
            else f"Parsed {len(entries)} entries"
        )
        return ParseRSSOutcome(
            success=True,
            message=message,
            total=total,
            entries=entries,
        )

    async def resolve_magnet(self, magnet: str, metadata_timeout: int = 30) -> Any:
        return await self._resolve_magnet(magnet, metadata_timeout=metadata_timeout)

    async def resolve_torrent(self, url: str) -> Any:
        return await self._resolve_torrent(url)

    async def _enrich_release(self, release: AnimeRelease) -> None:
        try:
            parse_results = await self._metadata_parser.parse([release])
            validated_results = await self._metadata_validator.validate(parse_results)
            parse_result: ParseResult = validated_results[0]
            if parse_result.success and parse_result.result:
                meta = parse_result.result
                release.anime_name = meta.anime_name
                release.season = meta.season
                release.episode = meta.episode
                release.quality = meta.quality
                release.fansub = meta.fansub
                release.languages = meta.languages
                release.version = meta.version
        except Exception as e:
            logger.warning(f"Metadata parsing failed for {release.title}: {e}")
