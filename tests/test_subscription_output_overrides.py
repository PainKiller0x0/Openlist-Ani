import unittest

from openlist_ani.domain.anime_release import AnimeRelease
from openlist_ani.domain.anime_release.naming import (
    ReleaseDirectoryPlanner,
    ReleaseFilenamePlanner,
)
from openlist_ani.domain.download_task.memento import (
    _release_from_dict,
    _release_to_dict,
)
from openlist_ani.adapters.inbound.http.schema import RSSPreviewRequest
from openlist_ani.adapters.outbound.feed_sources.subscription_reader import ReleaseFeedReader
from openlist_ani.adapters.outbound.configuration.settings import RSSSubscription


class SubscriptionOutputOverrideTests(unittest.TestCase):
    def test_preview_request_accepts_output_overrides(self):
        request = RSSPreviewRequest(
            url="https://example.invalid/feed.xml",
            filename_season_override=1,
            directory_season_override=7,
        )

        self.assertEqual(request.filename_season_override, 1)
        self.assertEqual(request.directory_season_override, 7)

    def test_existing_subscription_config_defaults_overrides_to_auto(self):
        subscription = RSSSubscription(url="https://example.invalid/feed.xml")

        self.assertEqual(subscription.filename_season_override, 0)
        self.assertEqual(subscription.directory_season_override, 0)

    def test_feed_reader_applies_subscription_overrides(self):
        feed_url = "https://example.invalid/feed.xml"
        release = AnimeRelease(
            title="RSS release",
            download_url="https://example.invalid/episode.mkv",
        )
        reader = ReleaseFeedReader(
            urls=[feed_url],
            filename_season_overrides={feed_url: 1},
            directory_season_overrides={feed_url: 7},
        )

        result = reader._collect_entries([(feed_url, [release])])

        self.assertEqual(result, [release])
        self.assertEqual(release.filename_season_override, 1)
        self.assertEqual(release.directory_season_override, 7)

    def test_filename_season_and_archive_season_can_differ(self):
        release = AnimeRelease(
            title="RSS release",
            download_url="https://example.invalid/episode.mkv",
            anime_name="飙马野郎 JOJO的奇妙冒险(2026)",
            download_directory_name_override="JOJO的奇妙冒险 (2012)",
            season=7,
            episode=3,
            filename_season_override=1,
            directory_season_override=7,
        )

        self.assertEqual(
            ReleaseFilenamePlanner("{anime_name} - S{season:02d}E{episode:02d}").stem(release),
            "飙马野郎 JOJO的奇妙冒险(2026) - S01E03",
        )
        self.assertEqual(
            ReleaseDirectoryPlanner().target_directory_path("/迅雷/videos/番", release),
            "/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 7",
        )

    def test_missing_overrides_keep_current_detected_naming(self):
        release = AnimeRelease(
            title="RSS release",
            download_url="https://example.invalid/episode.mkv",
            anime_name="JOJO的奇妙冒险 (2012)",
            season=7,
            episode=3,
        )

        self.assertEqual(
            ReleaseFilenamePlanner("{anime_name} - S{season:02d}E{episode:02d}").stem(release),
            "JOJO的奇妙冒险 (2012) - S07E03",
        )
        self.assertEqual(
            ReleaseDirectoryPlanner().target_directory_path("/迅雷/videos/番", release),
            "/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 7",
        )

    def test_download_memento_preserves_output_overrides(self):
        release = AnimeRelease(
            title="RSS release",
            download_url="https://example.invalid/episode.mkv",
            anime_name="Series",
            season=7,
            episode=3,
            filename_season_override=1,
            directory_season_override=7,
        )

        restored = _release_from_dict(_release_to_dict(release))

        self.assertEqual(restored.filename_season_override, 1)
        self.assertEqual(restored.directory_season_override, 7)


if __name__ == "__main__":
    unittest.main()
