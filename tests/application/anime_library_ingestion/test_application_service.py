from openlist_ani.application.anime_library_ingestion.application_service import (
    AnimeLibraryApplicationService,
    ParseRSSOutcome,
)
from openlist_ani.application.anime_library_ingestion.settings import (
    AnimeLibraryIngestionSettings,
)
from openlist_ani.application.anime_library_ingestion.models import (
    ParseResult,
    ReleaseTitleParseResult,
)
from openlist_ani.domain.anime_release import AnimeRelease, LanguageType
from openlist_ani.domain.download_task.memento import TaskMemento
from openlist_ani.domain.download_task.task import DownloadState


class LatestEpisodeRepository:
    async def find_latest_episodes(self, anime_names):
        self.anime_names = anime_names
        return {"Test Anime": 7}


class SeasonAwareLatestEpisodeRepository:
    async def find_latest_episodes(self, anime_names, *, seasons_by_name=None):
        self.seasons_by_name = seasons_by_name
        return {"Test Anime (2021)": 19}


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


async def test_list_rss_subscriptions_passes_season_for_normalized_latest_lookup():
    repository = SeasonAwareLatestEpisodeRepository()
    service = object.__new__(AnimeLibraryApplicationService)
    service._anime_library_repository = repository
    service._get_rss_subscriptions = lambda: [
        {
            "url": "https://example.test/rss",
            "name": "Test Anime (2021)",
            "anime_name": "Test Anime (2021)",
            "season": 2,
        }
    ]
    service._settings = AnimeLibraryIngestionSettings(
        download_path="/迅雷/videos/番",
        rename_format="{anime_name} S{season:02d}E{episode:02d}",
        rss_interval_seconds=300,
    )

    subscriptions = await service.list_rss_subscriptions()

    assert repository.seasons_by_name == {"Test Anime (2021)": 2}
    assert subscriptions[0]["latest_episode"] == 19


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


async def test_preview_applies_episode_offset_to_rename_and_directory():
    release = AnimeRelease(
        title="关于我转生变成史莱姆这档事 第四季 - 73",
        download_url="magnet:?xt=urn:btih:test",
    )
    parsed_result = ParseResult(
        success=True,
        result=ReleaseTitleParseResult(
            anime_name="关于我转生变成史莱姆这档事",
            season=4,
            episode=73,
            languages=[LanguageType.CHT],
            version=1,
        ),
        release_title=release.title,
    )

    service = object.__new__(AnimeLibraryApplicationService)
    service._settings = AnimeLibraryIngestionSettings(
        download_path="/迅雷/videos/番",
        rename_format="{anime_name} - S{season:02d}E{episode:02d}",
        rss_interval_seconds=300,
    )
    service.parse_rss = lambda url, enrich_metadata=False: _parsed_rss(release)
    service.get_global_exclude_patterns = lambda: []
    service._parse_preview_metadata = lambda entries: _parsed_metadata(parsed_result)
    service.resolve_rss_subscription = lambda *args, **kwargs: _preview_metadata()

    preview = await service.preview_rss_subscription(
        "https://example.test/rss",
        preferred_episode_offset=72,
    )

    assert preview["entries"][0]["episode"] == 1
    assert preview["entries"][0]["rename_preview"].endswith("S04E01")
    assert preview["entries"][0]["download_directory"].endswith("/Season 4")


async def test_preview_allows_special_episode_without_offset():
    release = AnimeRelease(
        title="[LoliHouse] 攻壳机动队 SAC_2045 全集",
        download_url="https://example.test/collection.torrent",
    )
    parsed_result = ParseResult(
        success=True,
        result=ReleaseTitleParseResult(
            anime_name="攻壳机动队 SAC_2045",
            season=0,
            episode=0,
            languages=[LanguageType.CHS],
            version=1,
        ),
        release_title=release.title,
    )

    service = object.__new__(AnimeLibraryApplicationService)
    service._settings = AnimeLibraryIngestionSettings(
        download_path="/迅雷/videos/番",
        rename_format="{anime_name} - S{season:02d}E{episode:02d}",
        rss_interval_seconds=300,
    )
    service.parse_rss = lambda url, enrich_metadata=False: _parsed_rss(release)
    service.get_global_exclude_patterns = lambda: []
    service._parse_preview_metadata = lambda entries: _parsed_metadata(parsed_result)
    service.resolve_rss_subscription = lambda *args, **kwargs: _preview_metadata()

    preview = await service.preview_rss_subscription("https://example.test/rss")

    assert preview["success"] is True
    assert preview["entries"][0]["episode"] == 0


def test_archive_failed_download_hides_task_without_deleting_it():
    task = TaskMemento(
        task_id="failed-archive",
        state=DownloadState.FAILED,
        release=AnimeRelease(
            title="[ANi] Test Anime - 01 [1080p]",
            download_url="magnet:?xt=urn:btih:failed-archive",
        ),
        base_path="/anime",
    )

    class Coordinator:
        def __init__(self):
            self.saved = []

        def get_task(self, task_id):
            return task if task_id == task.task_id else None

        def save(self, value):
            self.saved.append(value)

    class Pipeline:
        def __init__(self):
            self.task_coordinator = Coordinator()

    service = object.__new__(AnimeLibraryApplicationService)
    service._pipeline = Pipeline()

    success, message = service.archive_failed_download(task.task_id)

    assert success is True
    assert message == "失败任务已存档"
    assert task.archived is True
    assert service._pipeline.task_coordinator.saved == [task]


async def test_preview_clamps_invalid_llm_season_to_tmdb_season_one():
    release = AnimeRelease(
        title="穹庐下的魔女 - 02",
        download_url="magnet:?xt=urn:btih:one-season",
    )
    parsed_result = ParseResult(
        success=True,
        result=ReleaseTitleParseResult(
            anime_name="穹庐下的魔女",
            season=2,
            episode=2,
            tmdb_id=288971,
            languages=[LanguageType.CHS],
            version=1,
        ),
        release_title=release.title,
    )

    service = object.__new__(AnimeLibraryApplicationService)
    service._settings = AnimeLibraryIngestionSettings(
        download_path="/迅雷/videos/番",
        rename_format="{anime_name} - S{season:02d}E{episode:02d}",
        rss_interval_seconds=300,
    )
    service.parse_rss = lambda url, enrich_metadata=False: _parsed_rss(release)
    service.get_global_exclude_patterns = lambda: []
    service._parse_preview_metadata = lambda entries: _parsed_metadata(parsed_result)
    async def resolve_subscription(*args, **kwargs):
        return {
            "name": "穹庐下的魔女 (2026)",
            "tmdb_id": 288971,
            "poster_url": "",
            "seasons": [{"season_number": 1, "episode_count": 12}],
        }

    service.resolve_rss_subscription = resolve_subscription

    preview = await service.preview_rss_subscription("https://example.test/rss")

    assert preview["entries"][0]["season"] == 1
    assert preview["entries"][0]["rename_preview"].endswith("S01E02")


async def _parsed_rss(release):
    return ParseRSSOutcome(True, "ok", total=1, entries=[release])


async def _parsed_metadata(result):
    return [result]


async def _preview_metadata():
    return {"name": "关于我转生变成史莱姆这档事 (2025)", "tmdb_id": None, "poster_url": ""}


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
