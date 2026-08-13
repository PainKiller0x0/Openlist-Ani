"""FastAPI router defining all backend API endpoints and the small web UI."""

import os
import re
import signal
import uuid
from pathlib import Path
from urllib.parse import unquote

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import FileResponse

from openlist_ani.adapters.outbound.configuration import config
from openlist_ani.logger import logger
from .schema import (
    AddRSSRequest,
    AddRSSResponse,
    CorrectRSSRequest,
    CreateDownloadRequest,
    CreateDownloadResponse,
    DownloadListResponse,
    DownloadTaskResponse,
    ParseRSSRequest,
    ParseRSSResponse,
    ResolveMagnetRequest,
    ResolveMagnetResponse,
    ResolveTorrentRequest,
    ResolveTorrentResponse,
    RestartResponse,
    GlobalRSSFilterRequest,
    RSSPreviewRequest,
    ToggleRSSRequest,
    UISettingsRequest,
)
from .service import BackendApiService
from openlist_ani.application.anime_library_ingestion.exclusions import (
    normalize_exclude_patterns,
)

router = APIRouter(prefix="/api")
UPLOAD_DIR = Path("data/uploads")
TORRENT_NAME = re.compile(r"^[0-9a-f]{32}\.torrent$")
MAX_TORRENT_BYTES = 50 * 1024 * 1024


def _internal_backend_host() -> str:
    host = config.backend.host.strip()
    return "127.0.0.1" if host in {"", "0.0.0.0", "::"} else host


@router.get("/ui/state")
async def ui_state() -> dict:
    """Return only the UI-safe runtime state needed by the built-in page."""
    svc = BackendApiService.get()
    subscriptions = [
        item
        for item in svc.list_rss_subscriptions()
        if str(item.get("url", ""))
    ]
    return {
        "rss_urls": [
            str(item["url"])
            for item in subscriptions
            if item.get("enabled", True)
        ],
        "rss_subscriptions": subscriptions,
        "global_exclude_patterns": svc.global_exclude_patterns(),
        "download_path": config.openlist.download_path,
        "rss_status": svc.rss_status(),
        "tasks": [task.model_dump(mode="json") for task in svc.list_downloads()],
    }


@router.get("/ui/settings")
async def ui_settings() -> dict:
    """Return settings safe for the browser settings dialog."""
    return {
        "global_exclude_patterns": BackendApiService.get().global_exclude_patterns(),
        "llm": {
            "provider_type": config.llm.provider_type,
            "base_url": config.llm.openai_base_url,
            "model": config.llm.openai_model,
            "api_key_configured": bool(config.llm.openai_api_key),
            "tmdb_language": config.llm.tmdb_language,
            "metadata_parser_provider": config.metadata_parser.provider,
        },
        "download_path": config.openlist.download_path,
        "rename_format": config.openlist.rename_format,
    }


@router.post("/ui/settings")
async def ui_update_settings(request: UISettingsRequest) -> dict:
    """Persist global filters and LLM settings from the browser dialog."""
    svc = BackendApiService.get()
    svc.update_global_exclude_patterns(request.global_exclude_patterns)
    config.update_llm_settings(
        provider_type=request.llm_provider_type,
        api_key=request.llm_api_key or None,
        base_url=request.llm_base_url,
        model=request.llm_model,
        tmdb_language=request.tmdb_language,
        metadata_parser_provider=request.metadata_parser_provider,
    )
    return {
        "success": True,
        "message": "设置已保存；LLM/解析器相关修改将在服务重启后生效。",
        "requires_restart": True,
    }


@router.post("/ui/scan")
async def ui_scan_now() -> dict[str, object]:
    """Run the RSS scanner immediately instead of waiting for its timer."""
    svc = BackendApiService.get()
    return await svc.scan_rss_now()


@router.post("/ui/rss")
async def ui_add_rss(request: AddRSSRequest) -> dict:
    """Validate, persist and activate a new RSS subscription."""
    if not request.confirmed:
        raise HTTPException(status_code=409, detail="请先完成 RSS 识别预览，再点击确认保存")
    svc = BackendApiService.get()
    parsed = await svc.parse_rss(request.url, limit=1)
    if not parsed.success:
        raise HTTPException(status_code=400, detail=parsed.message or "RSS 解析失败")

    metadata = await svc.resolve_rss_subscription(request.url, request.name)
    success, message, urls = svc.add_rss_subscription(
        request.url,
        name=str(metadata.get("name", "") or request.name).strip(),
        tmdb_id=metadata.get("tmdb_id"),
        poster_url=str(metadata.get("poster_url", "") or ""),
        exclude_patterns=normalize_exclude_patterns(request.exclude_patterns),
    )
    preview = parsed.entries[0].title if parsed.entries else ""
    if success:
        message = "RSS 已保存，追踪器已立即更新；新条目会按轮询周期自动下载。"
    return {
        "success": success,
        "message": message,
        "urls": urls,
        "preview": preview,
        "name": metadata.get("name", "") or request.name.strip(),
        "tmdb_id": metadata.get("tmdb_id"),
        "poster_url": metadata.get("poster_url", ""),
        "exclude_patterns": normalize_exclude_patterns(request.exclude_patterns),
    }


@router.post("/ui/rss/preview")
async def ui_preview_rss(request: RSSPreviewRequest) -> dict:
    """Fetch and identify an RSS without saving it."""
    svc = BackendApiService.get()
    preview = await svc.preview_rss_subscription(
        request.url,
        preferred_name=request.name,
        exclude_patterns=request.exclude_patterns,
    )
    if not preview.get("success"):
        raise HTTPException(
            status_code=400,
            detail=preview.get("message", "RSS 解析失败"),
        )
    return preview


@router.post("/ui/rss/filter")
async def ui_update_global_rss_filter(request: GlobalRSSFilterRequest) -> dict:
    """Update the global RSS title exclusion list."""
    svc = BackendApiService.get()
    return svc.update_global_exclude_patterns(request.exclude_patterns)


@router.post("/ui/rss/toggle")
async def ui_toggle_rss(request: ToggleRSSRequest) -> dict:
    """Pause or resume a persisted RSS subscription without deleting it."""
    svc = BackendApiService.get()
    success, message, urls = svc.update_rss_subscription(
        request.url, enabled=request.enabled
    )
    if not success:
        raise HTTPException(status_code=404, detail=message)
    return {"success": True, "message": message, "urls": urls}


@router.post("/ui/rss/exclude")
async def ui_update_rss_exclude(request: AddRSSRequest) -> dict:
    """Update only one subscription's title exclusions."""
    svc = BackendApiService.get()
    success, message, urls = svc.update_rss_subscription(
        request.url,
        exclude_patterns=normalize_exclude_patterns(request.exclude_patterns),
    )
    if not success:
        raise HTTPException(status_code=404, detail=message)
    return {
        "success": True,
        "message": "单个 RSS 排除规则已保存，下一次扫描立即生效",
        "urls": urls,
        "exclude_patterns": normalize_exclude_patterns(request.exclude_patterns),
    }


@router.post("/ui/rss/correct")
async def ui_correct_rss(request: CorrectRSSRequest) -> dict:
    """Correct an existing RSS URL and its metadata in one operation."""
    svc = BackendApiService.get()
    success, message, urls = svc.correct_rss_subscription(
        request.original_url,
        url=request.url,
        name=request.name,
        tmdb_id=request.tmdb_id,
        poster_url=request.poster_url,
        exclude_patterns=request.exclude_patterns,
    )
    if not success:
        raise HTTPException(status_code=409, detail=message)
    return {"success": True, "message": message, "urls": urls}


@router.post("/ui/rss/metadata")
async def ui_refresh_rss_metadata(request: AddRSSRequest) -> dict:
    """Re-identify an existing RSS subscription and persist its metadata."""
    svc = BackendApiService.get()
    result = await svc.refresh_rss_subscription(
        request.url,
        preferred_name=request.name,
        preferred_tmdb_id=request.tmdb_id,
    )
    if not result.get("success"):
        raise HTTPException(status_code=404, detail=result.get("error", "RSS 不存在"))
    return result


@router.delete("/ui/rss")
async def ui_remove_rss(request: AddRSSRequest) -> dict:
    """Remove an RSS source and stop monitoring it immediately."""
    svc = BackendApiService.get()
    success, message, urls = svc.remove_rss_url(request.url)
    if not success:
        raise HTTPException(status_code=404, detail=message)
    return {"success": True, "message": message, "urls": urls}


@router.get("/ui/uploads/{filename}", include_in_schema=False)
async def serve_uploaded_torrent(filename: str) -> FileResponse:
    """Serve an uploaded torrent to OpenList on the same machine."""
    filename = unquote(filename)
    if not TORRENT_NAME.fullmatch(filename):
        raise HTTPException(status_code=404, detail="Not found")
    target = (UPLOAD_DIR / filename).resolve()
    if target.parent != UPLOAD_DIR.resolve() or not target.is_file():
        raise HTTPException(status_code=404, detail="Not found")
    return FileResponse(target, media_type="application/x-bittorrent")


@router.post("/ui/torrent")
async def ui_upload_torrent(request: Request) -> dict:
    """Upload a torrent, resolve its title, and start the OpenList task."""
    content_length = int(request.headers.get("content-length", "0"))
    if content_length <= 0 or content_length > MAX_TORRENT_BYTES:
        raise HTTPException(status_code=400, detail="种子文件为空或超过 50 MB 限制")
    if not request.headers.get("content-type", "").lower().startswith(
        "application/x-bittorrent"
    ):
        raise HTTPException(status_code=400, detail="请上传 .torrent 文件")

    filename = unquote(request.headers.get("x-filename", ""))
    if not filename.lower().endswith(".torrent"):
        raise HTTPException(status_code=400, detail="只支持 .torrent 文件")
    blob = await request.body()
    if len(blob) > MAX_TORRENT_BYTES:
        raise HTTPException(status_code=400, detail="种子文件超过 50 MB 限制")

    UPLOAD_DIR.mkdir(parents=True, exist_ok=True)
    stored = f"{uuid.uuid4().hex}.torrent"
    target = UPLOAD_DIR / stored
    target.write_bytes(blob)
    internal_url = (
        f"http://{_internal_backend_host()}:{config.backend.port}"
        f"/api/ui/uploads/{stored}"
    )

    svc = BackendApiService.get()
    try:
        resolved = await svc.resolve_torrent(internal_url)
        if not resolved.success:
            raise HTTPException(status_code=400, detail=resolved.message or "种子解析失败")
        title = unquote(request.headers.get("x-title", "")).strip()
        title = title[:200] or (resolved.title or Path(filename).stem)[:200]
        success, message, task = await svc.create_download(internal_url, title)
        if not success:
            raise HTTPException(status_code=409, detail=message)
        return {
            "success": True,
            "message": f"已创建迅雷任务：{title}",
            "task": task.model_dump(mode="json") if task else None,
        }
    except Exception:
        target.unlink(missing_ok=True)
        raise


@router.post("/restart")
async def restart_service() -> RestartResponse:
    """Restart the application by sending SIGHUP to self."""
    logger.debug("Backend: Restart requested via API")
    os.kill(
        os.getpid(), signal.SIGHUP
    )  # noqa: S603 – intentional self-signal for graceful restart
    return RestartResponse(success=True, message="Restart signal sent")


@router.post("/rss")
async def add_rss_url(request: AddRSSRequest) -> AddRSSResponse:
    """Add a new RSS monitoring URL."""
    svc = BackendApiService.get()
    success, message, urls = svc.add_rss_url(request.url, name=request.name)
    return AddRSSResponse(success=success, message=message, urls=urls)


@router.post("/downloads")
async def create_download(request: CreateDownloadRequest) -> CreateDownloadResponse:
    """Create a new download task."""
    svc = BackendApiService.get()
    success, message, task = await svc.create_download(
        download_url=request.download_url,
        title=request.title,
    )
    return CreateDownloadResponse(success=success, message=message, task=task)


@router.get("/downloads")
async def list_downloads() -> DownloadListResponse:
    """Get all active download tasks."""
    svc = BackendApiService.get()
    tasks = svc.list_downloads()
    return DownloadListResponse(tasks=tasks, total=len(tasks))


@router.get(
    "/downloads/{task_id}",
    responses={404: {"description": "Task not found"}},
)
async def get_download(task_id: str) -> DownloadTaskResponse:
    """Get a specific download task's status and progress."""
    svc = BackendApiService.get()
    task = svc.get_download(task_id)
    if task is None:
        raise HTTPException(status_code=404, detail=f"Task not found: {task_id}")
    return task


@router.post("/parse_rss")
async def parse_rss(request: ParseRSSRequest) -> ParseRSSResponse:
    """Parse an RSS feed and return its release entries.

    Returns raw, un-enriched entries (title, download_url, fansub, etc.).
    The caller (assistant) decides which entries to enqueue via
    ``/api/downloads``.
    """
    svc = BackendApiService.get()
    return await svc.parse_rss(url=request.url, limit=request.limit)


@router.post("/resolve_magnet")
async def resolve_magnet(request: ResolveMagnetRequest) -> ResolveMagnetResponse:
    """Resolve a magnet URI to its real title and file list.

    Order of operations: ``dn=`` parameter → libtorrent metadata
    (DHT/peers, bounded by ``metadata_timeout``).  Detects collection
    releases via title-keyword matching so callers can refuse them.
    """
    svc = BackendApiService.get()
    return await svc.resolve_magnet(
        magnet=request.magnet, metadata_timeout=request.metadata_timeout
    )


@router.post("/resolve_torrent")
async def resolve_torrent(request: ResolveTorrentRequest) -> ResolveTorrentResponse:
    """Resolve a .torrent file URL to its real title and file list.

    Downloads the .torrent via HTTP(S) (size- and time-bounded), then
    parses the blob with libtorrent.  Mirrors ``/api/resolve_magnet``'s
    response shape so callers can share the same downstream pipeline.
    """
    svc = BackendApiService.get()
    return await svc.resolve_torrent(url=request.url)
