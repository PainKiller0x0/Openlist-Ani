from openlist_ani.application.anime_library_ingestion.application_service import (
    AnimeLibraryApplicationService,
)
from openlist_ani.application.anime_library_ingestion.settings import (
    AnimeLibraryIngestionSettings,
)


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
