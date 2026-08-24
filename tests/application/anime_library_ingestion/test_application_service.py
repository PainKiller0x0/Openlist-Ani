from openlist_ani.application.anime_library_ingestion.application_service import (
    AnimeLibraryApplicationService,
)
from openlist_ani.application.anime_library_ingestion.settings import (
    AnimeLibraryIngestionSettings,
)
from openlist_ani.application.anime_library_ingestion.models import (
    ParseResult,
    ReleaseTitleParseResult,
)
from openlist_ani.domain.anime_release import AnimeRelease, LanguageType


class LatestEpisodeRepository:
    async def find_latest_episodes(self, anime_names):
        self.anime_names = anime_names
        return {"Test Anime": 7}


async def test_list_rss_subscriptions_enriches_latest_episode():
    repository = LatestEpisodeRepository()
    service = object.__new__(AnimeLibraryApplicationService)
    service._anime_library_repository = repository
    service._get_rss_subscriptions = lambda: [
        {
            "url": "https://example.test/rss",
            "name": "Test Anime",
            "anime_name": "Test Anime",
            "season": 1,
        }
    ]
    service._settings = AnimeLibraryIngestionSettings(
        download_path="/迅雷/videos/番",
        rename_format="{anime_name} S{season:02d}E{episode:02d}",
        rss_interval_seconds=300,
    )

    subscriptions = await service.list_rss_subscriptions()

    assert repository.anime_names == ["Test Anime"]
    assert subscriptions[0]["latest_episode"] == 7
    assert subscriptions[0]["download_directory"] == "/迅雷/videos/番/Test Anime/Season 1"


async def test_preview_metadata_skips_authoritative_validation():
    class Parser:
        async def parse(self, entries):
            return [
                ParseResult(
                    success=True,
                    result=ReleaseTitleParseResult(
                        anime_name="Test Anime",
                        season=1,
                        episode=1,
                        languages=[LanguageType.CHS],
                        version=1,
                    ),
                )
                for _ in entries
            ]

    class Validator:
        def __init__(self):
            self.calls = 0

        async def validate(self, results):
            self.calls += 1
            raise AssertionError("preview must not run authoritative validation")

    validator = Validator()
    service = object.__new__(AnimeLibraryApplicationService)
    service._metadata_parser = Parser()
    service._metadata_validator = validator

    results = await service._parse_preview_metadata(
        [AnimeRelease(title="Test", download_url="https://example.test/test")]
    )

    assert results[0].result.anime_name == "Test Anime"
    assert validator.calls == 0


async def test_resolve_subscription_uses_parsed_name_for_tmdb_lookup():
    lookup_names = []

    async def lookup_tmdb_show(name):
        lookup_names.append(name)
        return {
            "id": 123,
            "name": "Test Anime",
            "first_air_date": "2024-01-01",
            "poster_url": "https://image.test/poster.jpg",
        }

    service = object.__new__(AnimeLibraryApplicationService)
    service._lookup_tmdb_show = lookup_tmdb_show
    service._lookup_tmdb_show_by_id = None

    result = ParseResult(
        success=True,
        result=ReleaseTitleParseResult(
            anime_name="Test Anime",
            season=4,
            episode=1,
            languages=[LanguageType.CHS],
            version=1,
        ),
    )

    metadata = await service.resolve_rss_subscription(
        "https://example.test/rss",
        preferred_name="Re: Test Anime 第四季 袭失篇",
        entries=[AnimeRelease(title="Test", download_url="https://example.test/test")],
        validated_results=[result],
    )

    assert lookup_names == ["Test Anime"]
    assert metadata["tmdb_id"] == 123
    assert metadata["poster_url"] == "https://image.test/poster.jpg"
