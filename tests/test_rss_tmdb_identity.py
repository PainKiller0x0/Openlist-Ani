import unittest

from openlist_ani.adapters.outbound.metadata_validator.pipeline import MetadataValidationPipeline
from openlist_ani.application.anime_library_ingestion.application_service import AnimeLibraryApplicationService
from openlist_ani.application.anime_library_ingestion.models import (
    EpisodeMapping,
    ParseResult,
    ReleaseTitleParseResult,
    TMDBMatch,
)
from openlist_ani.domain.anime_release import AnimeRelease

WRONG_TMDB_ID = 336654
CORRECT_TMDB_ID = 283938
CANONICAL_NAME = '学生会也有洞！ (2026)'


def parsed_result(title):
    return ParseResult(
        success=True,
        release_title=title,
        result=ReleaseTitleParseResult(
            anime_name='Seitokai ni mo Ana wa Aru',
            season=1,
            episode=1,
            languages=[],
            version=1,
            tmdb_id=WRONG_TMDB_ID,
        ),
    )


class Parser:
    async def parse(self, entries):
        return [parsed_result(entry.title) for entry in entries]


class CorrectingValidator:
    def __init__(self):
        self.received_ids = []

    async def validate_identity(self, results):
        self.received_ids = [item.result.tmdb_id for item in results if item.result]
        corrected = [item.model_copy(deep=True) for item in results]
        for item in corrected:
            if item.result:
                item.result.tmdb_id = CORRECT_TMDB_ID
                item.result.anime_name = CANONICAL_NAME
        return corrected


class UnresolvedValidator:
    async def validate_identity(self, results):
        unresolved = [item.model_copy(deep=True) for item in results]
        for item in unresolved:
            item.success = False
            item.error = 'TMDB match not found'
            item.result = None
        return unresolved


class RSSIdentityValidationTests(unittest.IsolatedAsyncioTestCase):
    def make_service(self):
        service = object.__new__(AnimeLibraryApplicationService)
        service._metadata_parser = Parser()
        service._metadata_validator = CorrectingValidator()
        service.looked_up_names = []
        service.looked_up_ids = []

        async def lookup_by_name(name):
            service.looked_up_names.append(name)
            return {
                'id': WRONG_TMDB_ID,
                'name': '女友的朋友 (2026)',
                'poster_url': 'https://example.invalid/wrong-poster.jpg',
            }

        service._lookup_tmdb_show = lookup_by_name

        async def lookup_by_id(tmdb_id):
            service.looked_up_ids.append(tmdb_id)
            return {
                'id': tmdb_id,
                'name': CANONICAL_NAME,
                'first_air_date': '2026-10-04',
                'poster_url': 'https://example.invalid/correct-poster.jpg',
            }

        service._lookup_tmdb_show_by_id = lookup_by_id
        return service

    async def test_preview_validates_nonempty_parser_tmdb_id(self):
        service = self.make_service()
        entry = AnimeRelease(title='Seitokai ni mo Ana wa Aru - 01', download_url='https://example.invalid/1')

        results = await service._parse_preview_metadata([entry])

        self.assertEqual(service._metadata_validator.received_ids, [WRONG_TMDB_ID])
        self.assertEqual(results[0].result.tmdb_id, CORRECT_TMDB_ID)

    async def test_subscription_resolution_uses_corrected_tmdb_id(self):
        service = self.make_service()
        entry = AnimeRelease(title='Seitokai ni mo Ana wa Aru - 01', download_url='https://example.invalid/1')

        metadata = await service.resolve_rss_subscription('https://example.invalid/rss', entries=[entry])

        self.assertEqual(service._metadata_validator.received_ids, [WRONG_TMDB_ID])
        self.assertEqual(service.looked_up_ids, [CORRECT_TMDB_ID])
        self.assertEqual(service.looked_up_names, [])
        self.assertEqual(metadata['tmdb_id'], CORRECT_TMDB_ID)
        self.assertEqual(metadata['poster_url'], 'https://example.invalid/correct-poster.jpg')

    async def test_unresolved_identity_does_not_guess_tmdb_from_name(self):
        service = self.make_service()
        service._metadata_validator = UnresolvedValidator()
        entry = AnimeRelease(
            title='Seitokai ni mo Ana wa Aru - 01',
            download_url='https://example.invalid/1',
            anime_name='Seitokai ni mo Ana wa Aru',
        )

        metadata = await service.resolve_rss_subscription('https://example.invalid/rss', entries=[entry])

        self.assertEqual(service.looked_up_ids, [])
        self.assertEqual(service.looked_up_names, [])
        self.assertIsNone(metadata['tmdb_id'])
        self.assertEqual(metadata['poster_url'], '')

    async def test_pipeline_identity_only_replaces_parser_id_without_episode_lookup(self):
        class IdentityResolver:
            async def resolve(self, name):
                return TMDBMatch(tmdb_id=CORRECT_TMDB_ID, anime_name=CANONICAL_NAME)

            async def close(self):
                return None

        class EpisodeValidator:
            def __init__(self):
                self.received_ids = []

            async def validate(self, **kwargs):
                self.received_ids.append(kwargs['tmdb_id'])
                return EpisodeMapping(season=1, episode=1, strategy='test')

        episode_validator = EpisodeValidator()
        pipeline = MetadataValidationPipeline(
            identity_resolver=IdentityResolver(),
            episode_validator=episode_validator,
        )
        result = await pipeline.validate_identity([parsed_result('release')])

        self.assertEqual(result[0].result.tmdb_id, CORRECT_TMDB_ID)
        self.assertEqual(result[0].result.anime_name, CANONICAL_NAME)
        self.assertEqual(episode_validator.received_ids, [])

        validated = await pipeline.validate([parsed_result('release')])

        self.assertEqual(episode_validator.received_ids, [CORRECT_TMDB_ID])
        self.assertTrue(validated[0].success)
        self.assertEqual(validated[0].result.tmdb_id, CORRECT_TMDB_ID)


if __name__ == '__main__':
    unittest.main()