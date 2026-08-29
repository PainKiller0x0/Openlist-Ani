"""Tests for SqliteAnimeLibraryRepository."""

import sqlite3

from openlist_ani.adapters.outbound.persistence import (
    SqliteAnimeLibraryRepository,
)
from openlist_ani.domain.anime_release import AnimeRelease, VideoQuality


async def test_repository_reuses_existing_resources_table(tmp_path):
    db_path = tmp_path / "data.db"
    title = "[ANi] Already Downloaded - 01 [1080P]"
    with sqlite3.connect(db_path) as db:
        db.execute("""
            CREATE TABLE resources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                title TEXT UNIQUE NOT NULL,
                anime_name TEXT,
                season INTEGER,
                episode INTEGER,
                fansub TEXT,
                quality TEXT,
                languages TEXT,
                version INTEGER,
                downloaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        """)
        db.execute(
            "INSERT INTO resources (url, title) VALUES (?, ?)",
            ("magnet:?xt=urn:btih:test", title),
        )
        db.commit()

    repository = SqliteAnimeLibraryRepository(db_path=db_path)
    await repository.init()

    assert await repository.is_downloaded(title)
    with sqlite3.connect(db_path) as db:
        tables = {
            row[0]
            for row in db.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
    assert "resources" in tables
    assert "anime_library_entries" not in tables


async def test_repository_filters_downloaded_titles_from_large_candidate_set(tmp_path):
    db_path = tmp_path / "data.db"
    repository = SqliteAnimeLibraryRepository(db_path=db_path)
    await repository.init()
    await repository.add_release(
        AnimeRelease(
            title="Already Downloaded",
            download_url="magnet:?xt=urn:btih:downloaded",
        )
    )

    candidates = ["Already Downloaded", *[f"Candidate {index}" for index in range(950)]]

    assert await repository.find_existing_titles(candidates) == {"Already Downloaded"}


async def test_repository_finds_releases_by_many_episode_keys(tmp_path):
    db_path = tmp_path / "data.db"
    repository = SqliteAnimeLibraryRepository(db_path=db_path)
    await repository.init()
    await repository.add_release(
        AnimeRelease(
            title="[ANi] Test Anime - 01 [1080p]",
            download_url="magnet:?xt=urn:btih:test",
            anime_name="Test Anime",
            season=1,
            episode=1,
            fansub="ANi",
            quality=VideoQuality.Q1080P,
        )
    )

    keys = [
        ("Test Anime", 1, 1),
        *[(f"Anime {index}", 1, 1) for index in range(1200)],
    ]

    records = await repository.find_releases_by_episodes(keys)

    assert records[("Test Anime", 1, 1)] == [
        {
            "fansub": "ANi",
            "quality": "1080p",
            "languages": "",
            "version": 1,
        }
    ]


async def test_repository_tracks_latest_episode_and_strict_rejections(tmp_path):
    db_path = tmp_path / "data.db"
    repository = SqliteAnimeLibraryRepository(db_path=db_path)
    await repository.init()
    await repository.add_release(
        AnimeRelease(
            title="Test Anime - 03",
            download_url="magnet:?xt=urn:btih:episode-3",
            anime_name="Test Anime",
            season=1,
            episode=3,
        )
    )
    await repository.add_release(
        AnimeRelease(
            title="Test Anime - 01",
            download_url="magnet:?xt=urn:btih:episode-1",
            anime_name="Test Anime",
            season=1,
            episode=1,
        )
    )

    assert await repository.find_latest_episodes(["Test Anime"]) == {
        "Test Anime": 3
    }


async def test_repository_matches_latest_episode_by_normalized_series_name_and_season(
    tmp_path,
):
    db_path = tmp_path / "data.db"
    repository = SqliteAnimeLibraryRepository(db_path=db_path)
    await repository.init()
    await repository.add_release(
        AnimeRelease(
            title="Slime - 91",
            download_url="magnet:?xt=urn:btih:slime-91",
            anime_name="关于我转生变成史莱姆这档事 第四季 (2026)",
            season=4,
            episode=19,
        )
    )

    assert await repository.find_latest_episodes(
        ["关于我转生变成史莱姆这档事(2018)"],
        seasons_by_name={"关于我转生变成史莱姆这档事(2018)": 4},
    ) == {"关于我转生变成史莱姆这档事(2018)": 19}

    rejected = AnimeRelease(
        title="Test Anime - 03 duplicate",
        download_url="magnet:?xt=urn:btih:rejected",
    )
    await repository.record_strict_rejections([rejected])
    assert await repository.find_strict_rejected_urls([rejected.download_url]) == {
        rejected.download_url
    }
