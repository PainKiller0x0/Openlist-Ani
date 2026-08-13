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
from openlist_ani.adapters.outbound.configuration.settings import PLACEHOLDER_RSS_URL
from openlist_ani.logger import logger
from .schema import (
    AddRSSRequest,
    AddRSSResponse,
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
)
from .service import BackendApiService

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
    return {
        "rss_urls": [
            url for url in config.rss.urls if url != PLACEHOLDER_RSS_URL
        ],
        "download_path": config.openlist.download_path,
        "rss_status": svc.rss_status(),
        "tasks": [task.model_dump(mode="json") for task in svc.list_downloads()],
    }


@router.post("/ui/scan")
async def ui_scan_now() -> dict[str, object]:
    """Run the RSS scanner immediately instead of waiting for its timer."""
    svc = BackendApiService.get()
    return await svc.scan_rss_now()


@router.post("/ui/rss")
async def ui_add_rss(request: AddRSSRequest) -> dict:
    """Validate, persist and activate a new RSS subscription."""
    svc = BackendApiService.get()
    parsed = await svc.parse_rss(request.url, limit=1)
    if not parsed.success:
        raise HTTPException(status_code=400, detail=parsed.message or "RSS 解析失败")

    success, message, urls = svc.add_rss_url(request.url)
    preview = parsed.entries[0].title if parsed.entries else ""
    if success:
        message = "RSS 已保存，追踪器已立即更新；新条目会按轮询周期自动下载。"
    return {"success": success, "message": message, "urls": urls, "preview": preview}


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
    success, message, urls = svc.add_rss_url(request.url)
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
