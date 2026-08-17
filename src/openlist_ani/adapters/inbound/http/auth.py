"""Small cookie-session authentication for the built-in OpenList-Ani UI."""

from __future__ import annotations

import base64
import binascii
import hashlib
import hmac
import os
import secrets
import time


SESSION_COOKIE = "op_ani_session"
SESSION_TTL_SECONDS = 30 * 24 * 60 * 60
PASSWORD_HASH_ENV = "OP_ANI_AUTH_PASSWORD_HASH"
USERNAME_ENV = "OP_ANI_AUTH_USER"
SECRET_ENV = "OP_ANI_AUTH_SECRET"


def configured() -> bool:
    """Return whether web authentication has been configured."""
    return bool(os.getenv(USERNAME_ENV, "").strip() and os.getenv(PASSWORD_HASH_ENV, "").strip())


def configured_username() -> str:
    return os.getenv(USERNAME_ENV, "").strip()


def verify_password(password: str) -> bool:
    """Verify a password against the PBKDF2-SHA256 value in the environment."""
    encoded = os.getenv(PASSWORD_HASH_ENV, "").strip()
    try:
        algorithm, iterations, salt_text, digest_text = encoded.split("$", 3)
        if algorithm != "pbkdf2_sha256":
            return False
        iterations_int = int(iterations)
        if not 10_000 <= iterations_int <= 2_000_000:
            return False
        salt = base64.urlsafe_b64decode(salt_text + "=" * (-len(salt_text) % 4))
        expected = base64.urlsafe_b64decode(digest_text + "=" * (-len(digest_text) % 4))
    except (TypeError, ValueError, UnicodeDecodeError, binascii.Error):
        return False

    try:
        actual = hashlib.pbkdf2_hmac(
            "sha256", password.encode("utf-8"), salt, iterations_int
        )
    except (TypeError, ValueError):
        return False
    return hmac.compare_digest(actual, expected)


def issue_session(username: str) -> str:
    """Create a signed, stateless session token."""
    expires_at = int(time.time()) + SESSION_TTL_SECONDS
    payload = f"{username}|{expires_at}|{secrets.token_urlsafe(12)}"
    signature = hmac.new(_session_secret(), payload.encode("utf-8"), hashlib.sha256).digest()
    return f"{payload}.{_b64(signature)}"


def session_username(token: str | None) -> str | None:
    """Validate a session token and return its username when it is current."""
    if not token or "." not in token:
        return None
    payload, signature_text = token.rsplit(".", 1)
    try:
        signature = _unb64(signature_text)
        username, expires_text, _nonce = payload.split("|", 2)
        expires_at = int(expires_text)
    except (TypeError, ValueError, UnicodeDecodeError, binascii.Error):
        return None
    expected = hmac.new(_session_secret(), payload.encode("utf-8"), hashlib.sha256).digest()
    if expires_at <= int(time.time()) or not hmac.compare_digest(signature, expected):
        return None
    if username != configured_username():
        return None
    return username


def _session_secret() -> bytes:
    configured_secret = os.getenv(SECRET_ENV, "").strip()
    if configured_secret:
        return configured_secret.encode("utf-8")
    # A missing explicit secret is still safe enough for a single-process
    # deployment: changing the password hash invalidates all old sessions.
    return os.getenv(PASSWORD_HASH_ENV, "").encode("utf-8")


def _b64(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).decode("ascii").rstrip("=")


def _unb64(value: str) -> bytes:
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))
