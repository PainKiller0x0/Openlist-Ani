import asyncio

from openlist_ani.application.anime_library_ingestion.exclusions import (
    filter_releases_by_title,
)
from openlist_ani.domain.anime_release import AnimeRelease
from openlist_ani.logger import logger

from .feed_source import FeedSource
from .factory import FeedSourceFactory


class ReleaseFeedReader:
    """Reads releases from configured feed URLs.

    This adapter owns external feed parsing. Application-level duplicate
    filtering is handled by the anime library ingestion use case.
    """

    def __init__(
        self,
        urls: list[str],
        factory: FeedSourceFactory | None = None,
        exclusion_patterns: dict[str, list[str]] | None = None,
        global_exclusion_patterns: list[str] | None = None,
        anime_names: dict[str, str] | None = None,
        download_directory_names: dict[str, str] | None = None,
    ) -> None:
        self._urls = list(dict.fromkeys(urls))
        self._factory = factory or FeedSourceFactory()
        self._exclusion_patterns = dict(exclusion_patterns or {})
        self._global_exclusion_patterns = list(global_exclusion_patterns or [])
        self._anime_names = dict(anime_names or {})
        self._download_directory_names = dict(download_directory_names or {})

    def set_urls(self, urls: list[str]) -> None:
        """Replace monitored URLs without restarting the backend process."""
        self._urls = list(dict.fromkeys(urls))

    def set_exclusion_patterns(self, patterns_by_url: dict[str, list[str]]) -> None:
        """Replace per-subscription title exclusions at runtime."""
        self._exclusion_patterns = {
            url: list(patterns)
            for url, patterns in patterns_by_url.items()
            if patterns
        }

    def set_global_exclusion_patterns(self, patterns: list[str]) -> None:
        """Replace global title exclusions at runtime."""
        self._global_exclusion_patterns = list(patterns)

    def set_anime_names(self, names_by_url: dict[str, str]) -> None:
        """Replace explicit per-subscription anime-name overrides."""
        self._anime_names = {
            url: name.strip()
            for url, name in names_by_url.items()
            if name and name.strip()
        }

    def set_download_directory_names(self, names_by_url: dict[str, str]) -> None:
        """Replace explicit per-subscription download directory names."""
        self._download_directory_names = {
            url: name.strip()
            for url, name in names_by_url.items()
            if name and name.strip()
        }

    async def fetch_new_releases(self) -> list[AnimeRelease]:
        """Fetch releases from all configured feeds."""
        if not self._urls:
            return []

        fetches = self._build_fetch_tasks(self._urls)
        if not fetches:
            return []

        results = await asyncio.gather(
            *(task for _, task in fetches), return_exceptions=True
        )
        return self._collect_entries(list(zip((url for url, _ in fetches), results)))

    def _get_feed_source(self, url: str) -> FeedSource | None:
        """Get appropriate handler using FeedSourceFactory."""
        try:
            return self._factory.create(url)
        except Exception as e:
            logger.warning(f"Failed to create handler for URL {url}: {e}")
            return None

    def _build_fetch_tasks(self, urls: list[str]) -> list[tuple[str, object]]:
        """Build RSS fetch coroutine tasks for configured URLs."""
        tasks = []
        for url in urls:
            handler = self._get_feed_source(url)
            if handler is None:
                continue
            tasks.append((url, handler.fetch_feed(url)))
        return tasks

    def _collect_entries(self, results: list[tuple[str, object]]) -> list[AnimeRelease]:
        """Collect valid entries from fetched RSS results."""
        new_entries: list[AnimeRelease] = []
        for url, result in results:
            if not self._is_valid_feed_result(url, result):
                continue

            entries = [entry for entry in result if entry.download_url]
            accepted, excluded = filter_releases_by_title(
                entries,
                [
                    *self._global_exclusion_patterns,
                    *self._exclusion_patterns.get(url, []),
                ],
            )
            if excluded:
                logger.info(
                    f"RSS source excluded {len(excluded)} entr{'y' if len(excluded) == 1 else 'ies'} "
                    "by per-subscription rules"
                )
            for entry in accepted:
                if not entry.download_url:
                    continue
                entry.source_url = url
                entry.anime_name_override = self._anime_names.get(url)
                entry.download_directory_name_override = self._download_directory_names.get(url)
                new_entries.append(entry)
        return new_entries

    def _is_valid_feed_result(self, url: str, result: object) -> bool:
        """Validate a single fetch result and log errors if needed."""
        if isinstance(result, Exception):
            logger.warning(
                f"RSS source failed for {url}; continuing with other sources: {result}"
            )
            return False

        if not isinstance(result, list):
            logger.warning(f"Unexpected RSS fetch result for {url}: {result}")
            return False

        return True
