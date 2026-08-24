import base64
import hashlib

from fastapi.testclient import TestClient

from openlist_ani.adapters.inbound.http.app import create_app


def test_login_persists_session_cookie_over_http(monkeypatch):
    salt = b"test-salt"
    digest = hashlib.pbkdf2_hmac("sha256", b"test-pass", salt, 10_000)
    def encode(value: bytes) -> str:
        return base64.urlsafe_b64encode(value).decode().rstrip("=")

    monkeypatch.setenv("OP_ANI_AUTH_USER", "jack")
    monkeypatch.setenv(
        "OP_ANI_AUTH_PASSWORD_HASH",
        f"pbkdf2_sha256$10000${encode(salt)}${encode(digest)}",
    )
    monkeypatch.setenv("OP_ANI_AUTH_SECRET", "test-secret")

    client = TestClient(create_app(), base_url="http://testserver")
    login = client.post(
        "/api/auth/login",
        json={"username": "jack", "password": "test-pass"},
    )

    assert login.status_code == 200
    assert client.get("/api/auth/session").json() == {
        "authenticated": True,
        "username": "jack",
    }
