import asyncio
from abc import ABC, abstractmethod

import aiohttp
import feedparser

from openlist_ani.logger import logger
from openlist_ani.domain.anime_release import AnimeRelease


class FeedSource(ABC):
    """
    Abstract base class for RSS feed sources.
    """

    entry_concurrency: int | None = None

    async def fetch_feed(
        self, url: str, *, enrich_metadata: bool = True
    ) -> list[AnimeRelease]:
        """Fetch and parse RSS feed from a URL.

        Args:
            url: RSS feed URL

        Returns:
            List of parsed anime releases
        """
        timeout = aiohttp.ClientTimeout(total=30)

        try:
            async with aiohttp.ClientSession(
                timeout=timeout, trust_env=True
            ) as session:
                async with session.get(url) as response:
                    response.raise_for_status()
                    content = await response.text()

                    feed = feedparser.parse(content)

                    return await self._parse_entries(
                        feed.entries,
                        session,
                        enrich_metadata=enrich_metadata,
                    )
        except (aiohttp.ClientError, asyncio.TimeoutError) as e:
            logger.warning(f"RSS fetch failed for {url}: {e}")
            return []
        except Exception as e:
            logger.warning(f"Unexpected RSS fetch failure for {url}: {e}")
            return []

    @abstractmethod
    async def parse_entry(
        self, entry, session: aiohttp.ClientSession
    ) -> AnimeRelease | None:
        """Parse a single RSS entry.

        Args:
            entry: feedparser entry object
            session: Active aiohttp session for fetching additional data

        Returns:
            Parsed AnimeRelease or None if parsing fails
        """
        pass

    async def parse_entry_without_metadata(
        self, entry, session: aiohttp.ClientSession
    ) -> AnimeRelease | None:
        """Parse an entry without optional source-page enrichment."""
        return await self.parse_entry(entry, session)

    async def _parse_entries(
        self,
        entries,
        session: aiohttp.ClientSession,
        *,
        enrich_metadata: bool = True,
    ) -> list[AnimeRelease]:
        concurrency = self.entry_concurrency
        semaphore = (
            asyncio.Semaphore(concurrency)
            if concurrency is not None and concurrency > 0
            else None
        )

        async def parse_one(entry):
            parser = (
                self.parse_entry
                if enrich_metadata
                else self.parse_entry_without_metadata
            )
            if semaphore is None:
                return await parser(entry, session)
            async with semaphore:
                return await parser(entry, session)

        tasks = [parse_one(entry) for entry in entries]
        results = await asyncio.gather(*tasks, return_exceptions=True)

        parsed: list[AnimeRelease] = []
        for res in results:
            if isinstance(res, Exception):
                logger.warning(f"Failed to parse RSS entry: {res}")
                continue
            if res:
                parsed.append(res)

        return parsed
