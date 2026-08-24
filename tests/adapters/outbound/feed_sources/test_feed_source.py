from __future__ import annotations

import asyncio

from openlist_ani.adapters.outbound.feed_sources.feed_source import FeedSource
from openlist_ani.adapters.outbound.feed_sources.mikan import MikanFeedSource
from openlist_ani.domain.anime_release import AnimeRelease


class CountingFeedSource(FeedSource):
    entry_concurrency = 2

    def __init__(self) -> None:
        self.active = 0
        self.max_active = 0

    async def parse_entry(self, entry, session) -> AnimeRelease | None:
        self.active += 1
        self.max_active = max(self.max_active, self.active)
        await asyncio.sleep(0.01)
        self.active -= 1
        return AnimeRelease(title=f"Anime {entry}", download_url=f"magnet:?{entry}")


class FastPreviewFeedSource(FeedSource):
    def __init__(self) -> None:
        self.normal_calls = 0
        self.fast_calls = 0

    async def parse_entry(self, entry, session) -> AnimeRelease | None:
        self.normal_calls += 1
        return AnimeRelease(title=f"normal {entry}", download_url=f"magnet:?{entry}")

    async def parse_entry_without_metadata(self, entry, session) -> AnimeRelease | None:
        self.fast_calls += 1
        return AnimeRelease(title=f"fast {entry}", download_url=f"magnet:?{entry}")


async def test_parse_entries_respects_source_concurrency_limit():
    source = CountingFeedSource()

    entries = await source._parse_entries(list(range(6)), session=object())

    assert len(entries) == 6
    assert source.max_active <= 2


async def test_parse_entries_can_skip_optional_metadata_for_preview():
    source = FastPreviewFeedSource()

    entries = await source._parse_entries(
        [1], session=object(), enrich_metadata=False
    )

    assert [entry.title for entry in entries] == ["fast 1"]
    assert source.normal_calls == 0
    assert source.fast_calls == 1


async def test_mikan_preview_entry_does_not_fetch_detail_page():
    class Entry(dict):
        title = "[ANi] Test Anime - 01 [1080P][CHS]"
        link = "https://mikan.example/bangumi/1"

    source = MikanFeedSource()

    async def unexpected_metadata_fetch(*_args, **_kwargs):
        raise AssertionError("preview must not fetch the Mikan detail page")

    source._fetch_metadata = unexpected_metadata_fetch
    entry = Entry(
        enclosures=[
            {
                "type": "application/x-bittorrent",
                "href": "https://mikan.example/test.torrent",
            }
        ]
    )

    release = await source.parse_entry_without_metadata(entry, session=object())

    assert release is not None
    assert release.title == Entry.title
