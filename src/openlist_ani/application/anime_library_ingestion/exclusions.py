"""Shared RSS exclusion helpers.

The web UI accepts a human-friendly ``|`` separated value, while the
pipeline stores individual regular-expression patterns.  Keeping the
normalisation and matching rules here makes preview and background scans
behave identically.
"""

from __future__ import annotations

import re
from collections.abc import Iterable

from openlist_ani.domain.anime_release import AnimeRelease
from openlist_ani.logger import logger


def normalize_exclude_patterns(value: str | Iterable[str] | None) -> list[str]:
    """Turn UI/config values into trimmed, non-empty regex patterns.

    A pipe is the supported separator because RSS titles commonly contain
    commas and brackets.  Lists are also accepted for config/API callers.
    """
    if value is None:
        return []
    # Strings come from the UI and use ``|`` as a separator.  Lists come from
    # TOML/config/API and each element is already one regex; do not split
    # regex alternations such as ``(ABEMA|CR)`` inside those elements.
    if isinstance(value, str):
        values = _split_pattern_string(value)
    else:
        values = value
    patterns: list[str] = []
    for item in values:
        if not isinstance(item, str):
            continue
        patterns.append(item.strip())
    return list(dict.fromkeys(pattern for pattern in patterns if pattern))


def _split_pattern_string(value: str) -> list[str]:
    """Split top-level pipes without breaking regex alternations.

    This keeps both ``ABEMA|CR`` and ``(ABEMA|CR)`` useful.  Parentheses,
    character classes and escaped pipes are treated as part of a pattern.
    """
    parts: list[str] = []
    start = 0
    paren = bracket = brace = 0
    escaped = False
    for index, char in enumerate(value):
        if escaped:
            escaped = False
            continue
        if char == "\\":
            escaped = True
            continue
        if char == "(":
            paren += 1
        elif char == ")" and paren:
            paren -= 1
        elif char == "[":
            bracket += 1
        elif char == "]" and bracket:
            bracket -= 1
        elif char == "{":
            brace += 1
        elif char == "}" and brace:
            brace -= 1
        elif char == "|" and not (paren or bracket or brace):
            parts.append(value[start:index].strip())
            start = index + 1
    parts.append(value[start:].strip())
    return [part for part in parts if part]


def compile_exclude_patterns(patterns: Iterable[str] | None) -> list[re.Pattern[str]]:
    """Compile patterns, logging invalid entries and keeping scans alive."""
    compiled: list[re.Pattern[str]] = []
    for pattern in normalize_exclude_patterns(patterns):
        try:
            compiled.append(re.compile(pattern, re.IGNORECASE))
        except re.error as error:
            logger.warning(f"Ignoring invalid RSS exclusion pattern {pattern!r}: {error}")
    return compiled


def matched_exclude_pattern(
    title: str,
    patterns: Iterable[str] | None,
) -> str | None:
    """Return the first matching pattern, or ``None``."""
    return next(
        (pattern.pattern for pattern in compile_exclude_patterns(patterns) if pattern.search(title)),
        None,
    )


def filter_releases_by_title(
    releases: Iterable[AnimeRelease],
    patterns: Iterable[str] | None,
) -> tuple[list[AnimeRelease], list[tuple[AnimeRelease, str]]]:
    """Split releases into accepted and excluded groups."""
    compiled = compile_exclude_patterns(patterns)
    if not compiled:
        return list(releases), []

    accepted: list[AnimeRelease] = []
    excluded: list[tuple[AnimeRelease, str]] = []
    for release in releases:
        matched = next((p.pattern for p in compiled if p.search(release.title)), None)
        if matched is None:
            accepted.append(release)
        else:
            excluded.append((release, matched))
    return accepted, excluded
