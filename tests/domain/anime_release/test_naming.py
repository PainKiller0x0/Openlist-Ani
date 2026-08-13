from openlist_ani.domain.anime_release import (
    AnimeRelease,
    DEFAULT_RENAME_FORMAT,
    ReleaseDirectoryPlanner,
    ReleaseFilenamePlanner,
    format_series_name,
    validate_rename_format,
)


def test_format_series_name_appends_tmdb_year_once():
    assert format_series_name("幼女战记", "2017-01-06") == "幼女战记 (2017)"
    assert format_series_name("幼女战记 (2017)", "2017-01-06") == "幼女战记 (2017)"
    assert format_series_name("幼女战记", "") == "幼女战记"


def test_default_rename_format_is_episode_focused():
    release = AnimeRelease(
        title="Example",
        download_url="magnet:?xt=urn:btih:test",
        anime_name="示例番剧",
        season=2,
        episode=3,
    )

    assert (
        ReleaseFilenamePlanner(DEFAULT_RENAME_FORMAT).filename(release, "source.mkv")
        == "示例番剧 - S02E03.mkv"
    )


def test_directory_planner_can_override_only_the_directory_name():
    release = AnimeRelease(
        title="Example",
        download_url="magnet:?xt=urn:btih:test",
        anime_name="文件名番剧",
        download_directory_name_override="网盘目录番剧",
        season=1,
        episode=1,
    )

    assert (
        ReleaseDirectoryPlanner().target_directory_path("/迅雷/videos/番", release)
        == "/迅雷/videos/番/网盘目录番剧/Season 1"
    )


def test_validate_rename_format_rejects_unknown_fields_and_paths():
    assert validate_rename_format("{anime_name} - S{season:02d}E{episode:02d}")[0]
    assert validate_rename_format("{version}")[0]
    assert not validate_rename_format("{missing}")[0]
    assert not validate_rename_format("{anime_name}/S{season}")[0]
