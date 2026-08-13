"""Magnet-link resolution helpers (libtorrent-backed)."""

from .resolver import (
    LibtorrentMetadataClient,
    MagnetResolver,
    ResolveResult,
    TorrentFile,
    TorrentFileResolver,
    is_torrent_url,
    resolve_magnet,
    resolve_torrent,
    torrent_url_to_magnet,
)

__all__ = [
    "LibtorrentMetadataClient",
    "MagnetResolver",
    "ResolveResult",
    "TorrentFile",
    "TorrentFileResolver",
    "is_torrent_url",
    "resolve_magnet",
    "resolve_torrent",
    "torrent_url_to_magnet",
]
