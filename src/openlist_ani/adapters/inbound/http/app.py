"""
FastAPI application factory with lifespan management.
"""

from contextlib import asynccontextmanager
from collections.abc import AsyncIterator

from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles
from fastapi.responses import HTMLResponse, JSONResponse, RedirectResponse

from openlist_ani.logger import logger
from .auth import SESSION_COOKIE, configured, session_username
from .frontend import INDEX_HTML, LOGIN_HTML, WEB_DIR
from .router import router


@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncIterator[None]:
    """Manage application lifespan — startup and shutdown hooks."""
    logger.debug("Backend API server starting up")
    yield
    logger.debug("Backend API server shutting down")


def create_app() -> FastAPI:
    """Create and configure the FastAPI application.

    Returns:
        Configured FastAPI instance.
    """
    app = FastAPI(
        title="OpenList-Ani Backend",
        description="Internal API for anime download and RSS management",
        version="1.0.0",
        lifespan=lifespan,
    )
    app.mount("/assets", StaticFiles(directory=WEB_DIR / "assets"), name="assets")
    app.include_router(router)

    @app.middleware("http")
    async def web_auth_middleware(request, call_next):
        """Protect the UI and API while keeping the login handshake public."""
        if not configured():
            return await call_next(request)

        path = request.url.path
        public_paths = {
            "/login",
            "/api/auth/login",
            "/api/auth/session",
            "/api/auth/logout",
        }
        username = session_username(request.cookies.get(SESSION_COOKIE))
        if path in public_paths or path.startswith("/assets/") or username:
            return await call_next(request)
        if path.startswith("/api/"):
            return JSONResponse({"detail": "请先登录 op-ani"}, status_code=401)
        return RedirectResponse("/login", status_code=303)

    @app.get("/login", response_class=HTMLResponse, include_in_schema=False)
    async def login_page() -> HTMLResponse:
        return HTMLResponse(
            content=LOGIN_HTML,
            headers={"Cache-Control": "no-store, no-cache, must-revalidate, max-age=0"},
        )

    @app.get("/", response_class=HTMLResponse, include_in_schema=False)
    async def index() -> HTMLResponse:
        """Serve the user-facing RSS/torrent entry point."""
        # The page is embedded in the service and changes whenever the backend
        # is upgraded.  Prevent browsers and reverse proxies from keeping an
        # older UI after a deployment.
        return HTMLResponse(
            content=INDEX_HTML,
            headers={
                "Cache-Control": "no-store, no-cache, must-revalidate, max-age=0",
                "Pragma": "no-cache",
                "Expires": "0",
            },
        )

    return app
