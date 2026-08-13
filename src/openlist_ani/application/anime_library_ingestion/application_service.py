"""HTTP-facing application service for anime library operations."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass, replace
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
from openlist_ani.application.anime_library_ingestion.exclusions import (
    filter_releases_by_title,
    normalize_exclude_patterns,
)
from openlist_ani.domain.anime_release import (
    AnimeRelease,
    format_series_name,
    sanitize_filename,
)
from openlist_ani.domain.anime_release import (
    validate_rename_format,
    ReleaseDirectoryPlanner,
    ReleaseFilenamePlanner,
)
from openlist_ani.domain.download_task.memento import TaskMemento
from openlist_ani.domain.download_task.task import DownloadState
from openlist_ani.logger import logger


class ReleaseFeedSourcePort(Protocol):
    async def fetch_feed(self, url: str) -> list[AnimeRelease]: ...


class ReleaseFeedSourceFactoryPort(Protocol):
    def create(self, url: str) -> ReleaseFeedSourcePort: ...


ResolveMagnet = Callable[..., Awaitable[Any]]
ResolveTorrent = Callable[..., Awaitable[Any]]
ValidateOpenListPath = Callable[[str], Awaitable[tuple[bool, str]]]
ValidateOpenListURL = Callable[[str], Awaitable[tuple[bool, str]]]
ValidateOpenListPathAtURL = Callable[[str, str], Awaitable[tuple[bool, str]]]
UpdateOpenListURL = Callable[[str], None]
LookupTMDBShow = Callable[[str], Awaitable[dict[str, Any] | None]]
LookupTMDBShowByID = Callable[[int], Awaitable[dict[str, Any] | None]]


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
        get_global_exclude_patterns: Callable[[], list[str]] | None = None,
        update_global_exclude_patterns_func: Callable[[list[str]], None] | None = None,
        lookup_tmdb_show: LookupTMDBShow | None = None,
        lookup_tmdb_show_by_id: LookupTMDBShowByID | None = None,
        validate_openlist_path: ValidateOpenListPath | None = None,
        validate_openlist_url: ValidateOpenListURL | None = None,
        validate_openlist_path_at_url: ValidateOpenListPathAtURL | None = None,
        update_openlist_url: UpdateOpenListURL | None = None,
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
        self._get_global_exclude_patterns = get_global_exclude_patterns or (
            lambda: list(self._settings.metadata_filter.exclude_patterns)
        )
        self._update_global_exclude_patterns = update_global_exclude_patterns_func
        self._lookup_tmdb_show = lookup_tmdb_show
        self._lookup_tmdb_show_by_id = lookup_tmdb_show_by_id
        self._validate_openlist_path = validate_openlist_path
        self._validate_openlist_url = validate_openlist_url
        self._validate_openlist_path_at_url = validate_openlist_path_at_url
        self._update_openlist_url = update_openlist_url

    @property
    def pipeline(self) -> AnimeLibraryIngestionPipeline:
        return self._pipeline

    def add_rss_url(
        self,
        url: str,
        *,
        name: str = "",
        anime_name: str = "",
        tmdb_id: int | None = None,
        poster_url: str = "",
        season: int | None = None,
        exclude_patterns: list[str] | None = None,
    ) -> tuple[bool, str, list[str]]:
        current_urls = self._get_rss_urls()
        if url in current_urls:
            return False, f"URL already exists: {url}", current_urls

        try:
            self._add_rss_url(
                url,
                name=name,
                anime_name=anime_name,
                tmdb_id=tmdb_id,
                poster_url=poster_url,
                season=season,
                exclude_patterns=exclude_patterns,
            )
        except TypeError:
            # Keep old adapters and test doubles working while the richer
            # subscription metadata API rolls out.
            self._add_rss_url(url)
        updated_urls = self._get_rss_urls()
        self._sync_feed_reader(updated_urls)
        logger.info(f"Added RSS URL: {url}")
        return True, f"RSS URL added successfully: {url}", updated_urls

    def list_rss_subscriptions(self) -> list[dict[str, Any]]:
        subscriptions: list[dict[str, Any]] = []
        for item in self._get_rss_subscriptions():
            enriched = dict(item)
            name = sanitize_filename(
                str(
                    enriched.get("anime_name", "")
                    or enriched.get("name", "")
                    or "未命名订阅"
                )
            )
            season = int(enriched.get("season") or 1)
            enriched["download_directory"] = (
                f"{self._settings.download_path.rstrip('/')}/{name}/Season {season}"
            )
            subscriptions.append(enriched)
        return subscriptions

    async def resolve_rss_subscription(
        self,
        url: str,
        preferred_name: str = "",
        *,
        entries: list[AnimeRelease] | None = None,
        validated_results: list[ParseResult] | None = None,
    ) -> dict[str, Any]:
        """Infer a display name and optional TMDB poster from the feed."""
        if entries is None:
            parsed = await self.parse_rss(url, limit=5)
            if not parsed.success:
                return {"name": preferred_name.strip(), "error": parsed.message}
            entries = parsed.entries or []

        entries = entries[:5]
        if validated_results is None:
            validated_results = await self._parse_and_validate_metadata(entries)

        inferred_name = preferred_name.strip()
        tmdb_id: int | None = None
        authoritative_name = ""
        for release, parsed_result in zip(entries, validated_results):
            if not inferred_name and release.anime_name:
                inferred_name = release.anime_name.strip()
            result = parsed_result.result if parsed_result.success else None
            if result is None:
                continue
            inferred_name = inferred_name or result.anime_name.strip()
            authoritative_name = result.anime_name.strip()
            tmdb_id = result.tmdb_id
            if result.anime_name.strip() and not preferred_name.strip():
                inferred_name = result.anime_name.strip()
            if tmdb_id is not None:
                break

        tmdb = None
        if tmdb_id is not None and self._lookup_tmdb_show_by_id:
            try:
                tmdb = await self._lookup_tmdb_show_by_id(tmdb_id)
            except Exception as e:
                logger.debug(f"TMDB id lookup failed for {tmdb_id}: {e}")
        if tmdb is None and self._lookup_tmdb_show and inferred_name:
            try:
                tmdb = await self._lookup_tmdb_show(inferred_name)
            except Exception as e:
                logger.debug(f"TMDB poster lookup failed for {inferred_name}: {e}")
        if tmdb:
            tmdb_id = tmdb.get("id") or tmdb_id
            poster_url = tmdb.get("poster_url") or ""
            inferred_name = format_series_name(
                inferred_name or authoritative_name or str(tmdb.get("name") or ""),
                str(tmdb.get("first_air_date") or ""),
            )
        else:
            poster_url = ""
            if not preferred_name.strip() and authoritative_name:
                inferred_name = authoritative_name

        return {
            "name": inferred_name,
            "tmdb_id": tmdb_id,
            "poster_url": poster_url,
        }

    async def refresh_rss_subscription(
        self,
        url: str,
        *,
        preferred_name: str = "",
        preferred_tmdb_id: int | None = None,
    ) -> dict[str, Any]:
        """Resolve and persist metadata for an existing subscription."""
        metadata = await self.resolve_rss_subscription(url, preferred_name)
        tmdb_id = preferred_tmdb_id or metadata.get("tmdb_id")
        poster_url = str(metadata.get("poster_url", "") or "")
        resolved_name = str(metadata.get("name", "") or preferred_name).strip()

        if preferred_tmdb_id is not None and self._lookup_tmdb_show_by_id:
            details = await self._lookup_tmdb_show_by_id(preferred_tmdb_id)
            if details:
                resolved_name = str(details.get("name", "") or resolved_name)
                poster_url = str(details.get("poster_url", "") or poster_url)

        if not self._update_rss_subscription(
            url,
            name=resolved_name,
            tmdb_id=tmdb_id,
            poster_url=poster_url,
        ):
            return {"success": False, "error": f"RSS URL not found: {url}"}

        return {
            "success": True,
            "url": url,
            "name": resolved_name,
            "tmdb_id": tmdb_id,
            "poster_url": poster_url,
        }

    def update_rss_subscription(
        self,
        url: str,
        *,
        name: str | None = None,
        anime_name: str | None = None,
        enabled: bool | None = None,
        tmdb_id: int | None = None,
        poster_url: str | None = None,
        season: int | None = None,
        exclude_patterns: list[str] | None = None,
    ) -> tuple[bool, str, list[str]]:
        if not self._update_rss_subscription(
            url,
            name=name,
            anime_name=anime_name,
            enabled=enabled,
            tmdb_id=tmdb_id,
            poster_url=poster_url,
            season=season,
            exclude_patterns=exclude_patterns,
        ):
            return False, f"RSS URL not found: {url}", self._get_rss_urls()
        updated_urls = self._get_rss_urls()
        self._sync_feed_reader(updated_urls)
        state = "resumed" if enabled else "paused" if enabled is False else "updated"
        cancelled = 0
        if enabled is False:
            item = next(
                (item for item in self._get_rss_subscriptions() if item.get("url") == url),
                {},
            )
            cancelled = self._pipeline.cancel_downloads_for_source(
                url,
                anime_names={
                    str(item.get("name", "")),
                    str(item.get("anime_name", "")),
                },
                reason="RSS subscription paused",
            )
        suffix = f"；已停止 {cancelled} 个关联任务" if cancelled else ""
        return True, f"RSS subscription {state}: {url}{suffix}", updated_urls

    def correct_rss_subscription(
        self,
        original_url: str,
        *,
        url: str,
        name: str = "",
        anime_name: str = "",
        tmdb_id: int | None = None,
        poster_url: str = "",
        season: int | None = None,
        exclude_patterns: list[str] | None = None,
    ) -> tuple[bool, str, list[str]]:
        """Update a subscription in place, or replace it with a new URL."""
        original_url = original_url.strip()
        url = url.strip()
        subscriptions = self._get_rss_subscriptions()
        original = next(
            (item for item in subscriptions if str(item.get("url", "")) == original_url),
            None,
        )
        if original is None:
            return False, f"RSS URL not found: {original_url}", self._get_rss_urls()
        if not url:
            return False, "RSS URL cannot be empty", self._get_rss_urls()

        normalized = normalize_exclude_patterns(exclude_patterns)
        if url == original_url:
            effective_poster_url = poster_url.strip() or str(
                original.get("poster_url", "") or ""
            )
            success, message, urls = self.update_rss_subscription(
                original_url,
                name=name,
                anime_name=anime_name,
                tmdb_id=tmdb_id,
                poster_url=effective_poster_url,
                season=season,
                exclude_patterns=normalized,
            )
            return success, "RSS 已修正并保存" if success else message, urls

        if any(str(item.get("url", "")) == url for item in subscriptions):
            return False, f"RSS URL already exists: {url}", self._get_rss_urls()

        enabled = bool(original.get("enabled", True))
        added, message, _ = self.add_rss_url(
            url,
            name=name,
            anime_name=anime_name,
            tmdb_id=tmdb_id,
            poster_url=poster_url,
            season=season,
            exclude_patterns=normalized,
        )
        if not added:
            return False, message, self._get_rss_urls()
        if not enabled:
            self.update_rss_subscription(url, enabled=False)
        if not self._remove_rss_url(original_url):
            # Do not leave a duplicate source behind if the old record could
            # not be removed after the replacement was persisted.
            self._remove_rss_url(url)
            self._sync_feed_reader()
            return False, f"RSS URL could not be replaced: {original_url}", self._get_rss_urls()
        updated_urls = self._get_rss_urls()
        self._sync_feed_reader(updated_urls)
        return True, "RSS 已修正并替换保存", updated_urls

    def update_global_exclude_patterns(
        self, patterns: list[str]
    ) -> tuple[bool, str]:
        """Persist global title exclusions and apply them without restart."""
        normalized = normalize_exclude_patterns(patterns)
        if self._update_global_exclude_patterns is not None:
            self._update_global_exclude_patterns(normalized)
        # The settings object is shared with the running RSS stage.  Mutate
        # its list in place because the settings dataclass is intentionally
        # immutable while its runtime filter lists remain replaceable.
        self._settings.metadata_filter.exclude_patterns[:] = normalized
        self._sync_feed_reader()
        return True, "全局排除规则已保存，下一次扫描立即生效"

    def get_global_exclude_patterns(self) -> list[str]:
        return list(normalize_exclude_patterns(self._get_global_exclude_patterns()))

    async def validate_download_path(self, path: str) -> tuple[bool, str]:
        """Check a destination against the configured OpenList instance."""
        if self._validate_openlist_path is None:
            return False, "当前服务未连接 OpenList，无法验证下载目录"
        return await self._validate_openlist_path(path)

    async def validate_openlist_url(self, url: str) -> tuple[bool, str]:
        """Check a candidate OpenList endpoint before persisting it."""
        if self._validate_openlist_url is None:
            return False, "当前服务未提供 OpenList 地址验证能力"
        return await self._validate_openlist_url(url)

    async def validate_download_path_at_url(
        self, url: str, path: str
    ) -> tuple[bool, str]:
        """Validate a download path against a candidate OpenList endpoint."""
        if self._validate_openlist_path_at_url is None:
            return False, "当前服务未提供候选 OpenList 地址验证能力"
        return await self._validate_openlist_path_at_url(url, path)

    def validate_rename_format(self, rename_format: str) -> tuple[bool, str]:
        return validate_rename_format(rename_format)

    def update_runtime_openlist_settings(
        self, *, download_path: str, rename_format: str
    ) -> None:
        self._pipeline.update_runtime_openlist_settings(
            download_path=download_path,
            rename_format=rename_format,
        )

    def update_runtime_openlist_url(self, url: str) -> None:
        """Switch the shared OpenList client without restarting the service."""
        if self._update_openlist_url is not None:
            self._update_openlist_url(url)

    def update_runtime_rss_interval(self, interval_seconds: int) -> None:
        self._pipeline.update_runtime_rss_interval(interval_seconds)

    def update_runtime_max_download_retries(self, max_retries: int) -> None:
        self._pipeline.update_runtime_max_download_retries(max_retries)

    def _sync_feed_reader(self, updated_urls: list[str] | None = None) -> None:
        """Refresh active URLs and per-RSS filters on the live reader."""
        feed_reader = self._pipeline.feed_reader
        if feed_reader is None:
            return
        urls = updated_urls if updated_urls is not None else self._get_rss_urls()
        set_urls = getattr(feed_reader, "set_urls", None)
        if set_urls is not None:
            set_urls(urls)
        set_patterns = getattr(feed_reader, "set_exclusion_patterns", None)
        if set_patterns is not None:
            set_patterns({
                str(item.get("url", "")): normalize_exclude_patterns(
                    item.get("exclude_patterns", [])
                )
                for item in self._get_rss_subscriptions()
                if item.get("enabled", True) and item.get("url")
            })
        set_global_patterns = getattr(feed_reader, "set_global_exclusion_patterns", None)
        if set_global_patterns is not None:
            set_global_patterns(self.get_global_exclude_patterns())
        set_anime_names = getattr(feed_reader, "set_anime_names", None)
        if set_anime_names is not None:
            set_anime_names({
                str(item.get("url", "")): str(item.get("anime_name", ""))
                for item in self._get_rss_subscriptions()
                if item.get("enabled", True) and item.get("url")
            })

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
        item = next(
            (item for item in known_subscriptions if str(item.get("url", "")) == url),
            {},
        )
        cancelled = self._pipeline.cancel_downloads_for_source(
            url,
            anime_names={str(item.get("name", "")), str(item.get("anime_name", ""))},
            reason="RSS subscription deleted",
        )
        if not self._remove_rss_url(url):
            return False, f"RSS URL not found: {url}", current_urls
        updated_urls = self._get_rss_urls()
        self._sync_feed_reader(updated_urls)
        suffix = f"；已停止 {cancelled} 个关联任务" if cancelled else ""
        return True, f"RSS URL removed: {url}{suffix}", updated_urls

    async def preview_rss_subscription(
        self,
        url: str,
        *,
        preferred_name: str = "",
        preferred_anime_name: str = "",
        exclude_patterns: list[str] | None = None,
        entry_limit: int = 80,
    ) -> dict[str, Any]:
        """Fetch, identify and preview an RSS before it is saved."""
        parsed = await self.parse_rss(url)
        if not parsed.success:
            return {"success": False, "message": parsed.message}

        global_patterns = self.get_global_exclude_patterns()
        local_patterns = normalize_exclude_patterns(exclude_patterns)
        combined_patterns = list(dict.fromkeys(global_patterns + local_patterns))
        accepted, excluded = filter_releases_by_title(
            parsed.entries or [], combined_patterns
        )

        # Parse a bounded preview batch once.  This result is reused for the
        # subscription identity and rename/directory preview, avoiding the
        # previous N serial LLM calls plus a second RSS fetch.
        preview_limit = min(entry_limit, 24)
        preview_candidates = (accepted or (parsed.entries or []))[:preview_limit]
        validated_preview = await self._parse_and_validate_metadata(preview_candidates)
        metadata = await self.resolve_rss_subscription(
            url,
            preferred_name,
            entries=preview_candidates,
            validated_results=validated_preview,
        )

        validated_by_title = {
            result.release_title: result
            for result in validated_preview
            if result.release_title
        }
        filename_planner = ReleaseFilenamePlanner(self._settings.rename_format)
        directory_planner = ReleaseDirectoryPlanner()

        excluded_by_id = {
            id(release): pattern for release, pattern in excluded
        }

        def entry_payload(release: AnimeRelease) -> dict[str, Any]:
            parsed_result = validated_by_title.get(release.title)
            result = parsed_result.result if parsed_result and parsed_result.success else None
            enriched = (
                replace(
                    release,
                    anime_name=(
                        preferred_anime_name.strip() or result.anime_name
                    ),
                    anime_name_override=(
                        preferred_anime_name.strip() or None
                    ),
                    season=result.season,
                    episode=result.episode,
                    fansub=result.fansub,
                    quality=result.quality,
                    languages=result.languages,
                    version=result.version,
                )
                if result is not None
                else release
            )
            payload = {
                "title": release.title,
                "download_url": release.download_url,
                "anime_name": enriched.anime_name,
                "season": enriched.season,
                "episode": enriched.episode,
                "fansub": enriched.fansub,
                "quality": enriched.quality.value if enriched.quality else None,
                "languages": [lang.value for lang in (enriched.languages or [])],
                "excluded": id(release) in excluded_by_id,
                "matched_pattern": excluded_by_id.get(id(release)),
                "llm_parsed": result is not None,
            }
            if result is not None:
                payload["tmdb_id"] = result.tmdb_id
                payload["rename_preview"] = filename_planner.stem(enriched)
                payload["download_directory"] = directory_planner.target_directory_path(
                    self._settings.download_path, enriched
                )
            return payload

        ordered = [*accepted, *(release for release, _ in excluded)]
        return {
            "success": True,
            "message": "RSS 已识别，请检查排除后的下载预览",
            "url": url,
            "name": metadata.get("name", "") or preferred_name.strip(),
            "anime_name": preferred_anime_name.strip(),
            "tmdb_id": metadata.get("tmdb_id"),
            "poster_url": metadata.get("poster_url", ""),
            "download_path": self._settings.download_path,
            "rename_format": self._settings.rename_format,
            "global_exclude_patterns": global_patterns,
            "exclude_patterns": local_patterns,
            "total": len(parsed.entries or []),
            "included": len(accepted),
            "excluded": len(excluded),
            "entries": [entry_payload(release) for release in ordered[:entry_limit]],
            "truncated": len(ordered) > entry_limit,
        }

    async def _parse_and_validate_metadata(
        self, entries: list[AnimeRelease]
    ) -> list[ParseResult]:
        """Parse a preview batch, validating only when identity is missing.

        The LLM title parser already returns a TMDB id for the normal case.
        Re-running the full TMDB identity/episode pipeline for every preview
        entry adds several network/LLM round trips without changing the
        rename or directory preview.  Keep that pipeline as a correctness
        fallback for parsers that cannot provide an id.
        """
        if not entries:
            return []
        try:
            parsed = await self._metadata_parser.parse(entries)
            missing_identity = [
                result
                for result in parsed
                if result.success
                and result.result is not None
                and result.result.tmdb_id is None
            ]
            if not missing_identity:
                return parsed
            validated_missing = await self._metadata_validator.validate(missing_identity)
            validated_by_title = {
                result.release_title: result
                for result in validated_missing
                if result.release_title
            }
            return [
                validated_by_title.get(result.release_title, result)
                for result in parsed
            ]
        except Exception as error:
            logger.debug(f"RSS metadata preview failed: {error}")
            return [
                ParseResult(success=False, release_title=entry.title, error=str(error))
                for entry in entries
            ]

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
