import pytest

from openlist_ani.application.anime_library_ingestion.models import (
    ParseResult,
    ReleaseTitleParseResult,
)
from openlist_ani.application.anime_library_ingestion.stages import (
    RSSStage,
    _apply_episode_offset,
)
from openlist_ani.domain.anime_release import AnimeRelease


def test_episode_offset_converts_absolute_episode_to_season_local_episode():
    result = ParseResult(
        success=True,
        result=ReleaseTitleParseResult(
            anime_name="关于我转生变成史莱姆这档事",
            season=4,
            episode=73,
            languages=[],
            version=1,
        ),
    )

    _apply_episode_offset(result, 72)

    assert result.success is True
    assert result.result.episode == 1


def test_episode_offset_rejects_non_positive_episode():
    result = ParseResult(
        success=True,
        result=ReleaseTitleParseResult(
            anime_name="Test Anime",
            season=1,
            episode=72,
            languages=[],
            version=1,
        ),
    )

    _apply_episode_offset(result, 72)

    assert result.success is False
    assert result.result is None


@pytest.mark.asyncio
async def test_rss_stage_applies_episode_offset_before_validation():
    class Parser:
        async def parse(self, entries):
            return [
                ParseResult(
                    success=True,
                    result=ReleaseTitleParseResult(
                        anime_name="Test Anime",
                        season=4,
                        episode=73,
                        languages=[],
                        version=1,
                    ),
                )
                for _ in entries
            ]

    class Validator:
        def __init__(self):
            self.seen_episode = None

        async def validate(self, results):
            self.seen_episode = results[0].result.episode
            return results

    validator = Validator()
    stage = object.__new__(RSSStage)
    stage._metadata_parser = Parser()
    stage._metadata_validator = validator
    stage._metadata_cache = {}

    results = await stage._parse_metadata([
        AnimeRelease(
            title="Test Anime 第四季 - 73",
            download_url="magnet:?xt=urn:btih:test",
            episode_offset=72,
        )
    ])

    assert validator.seen_episode == 1
    assert results[0].result.episode == 1
