"""HTML entrypoints for the maintainable static OpenList-Ani frontend."""

from pathlib import Path


WEB_DIR = Path(__file__).with_name("web")


def _read_page(filename: str) -> str:
    return (WEB_DIR / filename).read_text(encoding="utf-8")


LOGIN_HTML = _read_page("login.html")
INDEX_HTML = _read_page("index.html")
