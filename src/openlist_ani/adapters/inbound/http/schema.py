"""
Pydantic request/response models for backend API.
"""

from pydantic import BaseModel, Field


class AddRSSRequest(BaseModel):
    """Request body for adding an RSS monitoring URL."""

    url: str = Field(..., description="RSS feed URL to monitor")
    name: str = Field(default="", max_length=200, description="Optional anime display name")
    anime_name: str = Field(default="", max_length=200, description="Optional download anime_name override")
    download_directory_name: str = Field(default="", max_length=200, description="Optional download directory name override")
    episode_offset: int = Field(default=0, ge=0, le=999, description="Subtract from parsed episode numbers for this RSS")
    tmdb_id: int | None = Field(default=None, description="Optional TMDB TV show id")
    season: int | None = Field(default=None, ge=0, le=99)
    exclude_patterns: str = Field(
        default="",
        max_length=2000,
        description="Pipe-separated title exclusion regex patterns",
    )
    confirmed: bool = Field(
        default=False,
        description="Whether the user confirmed the preview before saving",
    )


class RSSPreviewRequest(BaseModel):
    """Request body for previewing an RSS before persistence."""

    url: str = Field(..., description="RSS feed URL")
    name: str = Field(default="", max_length=200, description="Optional display name")
    anime_name: str = Field(default="", max_length=200, description="Optional download anime_name override")
    download_directory_name: str = Field(default="", max_length=200, description="Optional download directory name override")
    episode_offset: int = Field(default=0, ge=0, le=999, description="Subtract from parsed episode numbers for this RSS")
    exclude_patterns: str = Field(
        default="", max_length=2000, description="Pipe-separated title exclusions"
    )


class GlobalRSSFilterRequest(BaseModel):
    """Request body for the global RSS title exclusion list."""

    exclude_patterns: str = Field(
        default="", max_length=4000, description="Pipe-separated title exclusions"
    )


class ToggleRSSRequest(BaseModel):
    """Request body for pausing or resuming an RSS subscription."""

    url: str = Field(..., description="RSS feed URL")
    enabled: bool = Field(..., description="Whether the subscription should keep polling")


class CorrectRSSRequest(BaseModel):
    """Request body for correcting an existing RSS subscription."""

    original_url: str = Field(..., description="Current persisted RSS URL")
    url: str = Field(..., description="Corrected RSS URL")
    name: str = Field(default="", max_length=200)
    anime_name: str = Field(default="", max_length=200)
    download_directory_name: str = Field(default="", max_length=200)
    episode_offset: int = Field(default=0, ge=0, le=999)
    tmdb_id: int | None = Field(default=None)
    season: int | None = Field(default=None, ge=0, le=99)
    poster_url: str = Field(default="", max_length=1000)
    exclude_patterns: str = Field(default="", max_length=2000)


class UISettingsRequest(BaseModel):
    """Request body for the built-in settings dialog."""

    global_exclude_patterns: str = Field(default="", max_length=4000)
    openlist_url: str | None = Field(default=None, max_length=1000)
    llm_provider_type: str | None = Field(default=None, max_length=50)
    llm_api_key: str | None = Field(default=None, max_length=500)
    llm_base_url: str | None = Field(default=None, max_length=1000)
    llm_model: str | None = Field(default=None, max_length=200)
    tmdb_language: str | None = Field(default=None, max_length=30)
    metadata_parser_provider: str | None = Field(default=None, max_length=50)
    download_path: str | None = Field(default=None, max_length=1000)
    rename_format: str | None = Field(default=None, max_length=1000)
    poll_interval_seconds: int | None = Field(default=None, ge=60, le=86400)
    max_download_retries: int | None = Field(default=None, ge=0, le=100)
    mikan_base_url: str | None = Field(default=None, max_length=1000)


class MikanSearchRequest(BaseModel):
    """Request body for searching the configured Mikan-compatible site."""

    keyword: str = Field(..., min_length=1, max_length=200)
    base_url: str | None = Field(default=None, max_length=1000)


class MikanGroupsRequest(BaseModel):
    """Request body for listing subtitle groups for one Mikan bangumi."""

    bangumi_id: int = Field(..., ge=1)
    base_url: str | None = Field(default=None, max_length=1000)


class MikanRSSRequest(BaseModel):
    """Request body for building a group-specific Mikan RSS URL."""

    bangumi_id: int = Field(..., ge=1)
    subgroup_id: int | None = Field(default=None, ge=1)
    base_url: str | None = Field(default=None, max_length=1000)


class RSSSubscriptionResponse(BaseModel):
    url: str
    name: str = ""
    anime_name: str = ""
    download_directory_name: str = ""
    episode_offset: int = 0
    enabled: bool = True
    tmdb_id: int | None = None
    poster_url: str = ""


class AddRSSResponse(BaseModel):
    """Response for adding an RSS URL."""

    success: bool
    message: str
    urls: list[str] = Field(default_factory=list, description="Current RSS URL list")


class CreateDownloadRequest(BaseModel):
    """Request body for creating a new download task."""

    download_url: str = Field(..., description="Download URL (magnet/torrent link)")
    title: str = Field(..., description="Release title for identification")


class DownloadTaskResponse(BaseModel):
    """Response model for a single download task."""

    id: str
    title: str
    download_url: str
    state: str
    anime_name: str | None = None
    season: int | None = None
    episode: int | None = None
    fansub: str | None = None
    quality: str | None = None
    progress: float | None = None
    error_message: str | None = None
    retry_count: int = 0
    created_at: str
    updated_at: str
    started_at: str | None = None
    completed_at: str | None = None
    save_path: str = ""
    final_path: str | None = None


class DownloadListResponse(BaseModel):
    """Response model for listing all download tasks."""

    tasks: list[DownloadTaskResponse]
    total: int


class CreateDownloadResponse(BaseModel):
    """Response for creating a download task."""

    success: bool
    message: str
    task: DownloadTaskResponse | None = None


class RestartResponse(BaseModel):
    """Response for restart endpoint."""

    success: bool
    message: str


# ── parse_rss ────────────────────────────────────────────────────────


class ParseRSSRequest(BaseModel):
    """Request body for parsing an RSS feed."""

    url: str = Field(..., description="RSS feed URL to parse")
    limit: int | None = Field(
        default=None,
        description="Maximum number of entries to return (None = all)",
    )


class ParseRSSEntry(BaseModel):
    """A single release entry parsed from an RSS feed."""

    index: int = Field(..., description="0-based position in the feed")
    title: str
    download_url: str
    anime_name: str | None = None
    episode: int | None = None
    fansub: str | None = None
    quality: str | None = None
    languages: list[str] = Field(default_factory=list)


class ParseRSSResponse(BaseModel):
    """Response for parsing an RSS feed."""

    success: bool
    message: str
    total: int = 0
    entries: list[ParseRSSEntry] = Field(default_factory=list)


# ── resolve_magnet ───────────────────────────────────────────────────


class ResolveMagnetRequest(BaseModel):
    """Request body for resolving a magnet link to its title / files."""

    magnet: str = Field(..., description="Magnet URI (magnet:?xt=urn:btih:…)")
    metadata_timeout: int = Field(
        default=30,
        description="libtorrent metadata fetch budget, in seconds",
    )


class ResolveMagnetFile(BaseModel):
    """A single file inside the torrent's metadata."""

    name: str
    size: int = 0


class ResolveMagnetResponse(BaseModel):
    """Response for ``/api/resolve_magnet``.

    ``title`` may be ``None`` when both ``dn=`` and metadata fetch fail;
    callers must in that case ask the user for the release title rather
    than fabricate one.
    """

    success: bool
    message: str
    title: str | None = None
    source: str | None = Field(
        default=None,
        description="Where the title came from: 'dn' | 'metadata' | None",
    )
    file_count: int | None = None
    files: list[ResolveMagnetFile] = Field(default_factory=list)


# ── resolve_torrent ──────────────────────────────────────────────────


class ResolveTorrentRequest(BaseModel):
    """Request body for resolving a .torrent file URL to its title / files."""

    url: str = Field(..., description="HTTP(S) URL to a .torrent file")


class ResolveTorrentResponse(BaseModel):
    """Response for ``/api/resolve_torrent``.

    Mirrors :class:`ResolveMagnetResponse` so callers can share the
    same downstream pipeline.
    """

    success: bool
    message: str
    title: str | None = None
    source: str | None = Field(
        default=None,
        description="Where the title came from: 'torrent_file' | None",
    )
    file_count: int | None = None
    files: list[ResolveMagnetFile] = Field(default_factory=list)
