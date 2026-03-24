#!/usr/bin/env python3
from __future__ import annotations

import base64
import hashlib
import json
import os
import secrets
import stat
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Any

DEFAULT_MCP_SERVER_URL = "https://mcp.notion.com/mcp"
DEFAULT_PROTOCOL_VERSION = "2025-06-18"
APP_NAME = "notioncli"
LEGACY_APP_NAME = "notion-cli"


class HttpRequestError(RuntimeError):
    def __init__(self, status: int, body: str, headers: dict[str, str]) -> None:
        super().__init__(f"HTTP {status}: {body}")
        self.status = status
        self.body = body
        self.headers = headers


def _config_dir() -> Path:
    if raw := os.environ.get("XDG_CONFIG_HOME"):
        base = Path(raw)
        preferred = base / APP_NAME
        legacy = base / LEGACY_APP_NAME
        if not preferred.exists() and legacy.exists():
            return legacy
        return preferred
    preferred = Path.home() / ".config" / APP_NAME
    legacy = Path.home() / ".config" / LEGACY_APP_NAME
    if not preferred.exists() and legacy.exists():
        return legacy
    return preferred


def default_token_store_path() -> Path:
    return _config_dir() / "notion-mcp-oauth.json"


def default_pending_oauth_path() -> Path:
    return _config_dir() / "notion-mcp-oauth-pending.json"


def _http_request(
    url: str,
    *,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    json_body: dict[str, Any] | list[Any] | None = None,
    form_body: dict[str, str] | None = None,
    timeout: float = 30.0,
) -> tuple[int, dict[str, str], str]:
    request_headers = {
        "User-Agent": "Mozilla/5.0",
        "Accept": "application/json",
        **dict(headers or {}),
    }
    body: bytes | None = None

    if json_body is not None:
        body = json.dumps(json_body).encode("utf-8")
        request_headers.setdefault("Content-Type", "application/json")
        request_headers.setdefault("Accept", "application/json")
    elif form_body is not None:
        body = urllib.parse.urlencode(form_body).encode("utf-8")
        request_headers.setdefault(
            "Content-Type", "application/x-www-form-urlencoded"
        )
        request_headers.setdefault("Accept", "application/json")

    request = urllib.request.Request(url, data=body, method=method, headers=request_headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            content_type = response.headers.get("Content-Type", "")
            if "text/event-stream" in content_type:
                chunks: list[str] = []
                saw_data = False
                while True:
                    line = response.readline()
                    if not line:
                        break
                    decoded = line.decode("utf-8", errors="replace")
                    chunks.append(decoded)
                    if decoded.startswith("data:"):
                        saw_data = True
                    if saw_data and decoded.strip() == "":
                        break
                text = "".join(chunks)
            else:
                text = response.read().decode("utf-8")
            return response.status, dict(response.headers.items()), text
    except urllib.error.HTTPError as error:
        text = error.read().decode("utf-8", errors="replace")
        raise HttpRequestError(error.code, text, dict(error.headers.items())) from error


def _http_json(
    url: str,
    *,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    json_body: dict[str, Any] | list[Any] | None = None,
    form_body: dict[str, str] | None = None,
    timeout: float = 30.0,
) -> tuple[int, dict[str, str], Any]:
    status, response_headers, text = _http_request(
        url,
        method=method,
        headers=headers,
        json_body=json_body,
        form_body=form_body,
        timeout=timeout,
    )
    if not text.strip():
        return status, response_headers, None
    return status, response_headers, json.loads(text)


def discover_oauth_metadata(server_url: str = DEFAULT_MCP_SERVER_URL) -> dict[str, Any]:
    parsed = urllib.parse.urlparse(server_url)
    base_origin = f"{parsed.scheme}://{parsed.netloc}"
    candidates = []
    for candidate in (
        base_origin.rstrip("/") + "/.well-known/oauth-protected-resource",
        server_url.rstrip("/") + "/.well-known/oauth-protected-resource",
        urllib.parse.urljoin(server_url, "/.well-known/oauth-protected-resource"),
    ):
        if candidate not in candidates:
            candidates.append(candidate)

    protected_resource: dict[str, Any] | None = None
    errors: list[str] = []

    for candidate in candidates:
        try:
            _, _, payload = _http_json(candidate)
            if isinstance(payload, dict):
                protected_resource = payload
                break
        except Exception as error:  # noqa: BLE001
            errors.append(f"{candidate}: {error}")

    if protected_resource is None:
        joined = "; ".join(errors) if errors else "no candidate URLs attempted"
        raise RuntimeError(f"failed to discover protected resource metadata: {joined}")

    authorization_servers = protected_resource.get("authorization_servers")
    if not isinstance(authorization_servers, list) or not authorization_servers:
        raise RuntimeError(
            "protected resource metadata did not include any authorization servers"
        )

    auth_server_url = authorization_servers[0]
    metadata_url = auth_server_url.rstrip("/") + "/.well-known/oauth-authorization-server"
    _, _, metadata = _http_json(metadata_url)
    if not isinstance(metadata, dict):
        raise RuntimeError("authorization server metadata response was not a JSON object")

    metadata["_protected_resource_metadata"] = protected_resource
    metadata["_protected_resource_url"] = next(
        (
            candidate
            for candidate in candidates
            if candidate.rstrip("/") == base_origin.rstrip("/") + "/.well-known/oauth-protected-resource"
        ),
        candidates[0],
    )
    metadata["_auth_server_url"] = auth_server_url
    return metadata


def generate_code_verifier() -> str:
    return base64.urlsafe_b64encode(secrets.token_bytes(32)).decode("ascii").rstrip("=")


def generate_code_challenge(code_verifier: str) -> str:
    digest = hashlib.sha256(code_verifier.encode("ascii")).digest()
    return base64.urlsafe_b64encode(digest).decode("ascii").rstrip("=")


def _save_json_secure(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    path.chmod(stat.S_IRUSR | stat.S_IWUSR)


def load_token_data(path: Path | None = None) -> dict[str, Any]:
    token_path = path or default_token_store_path()
    if not token_path.exists():
        raise FileNotFoundError(
            f"token file not found at {token_path}; run scripts/notion_mcp_login.py first"
        )
    return json.loads(token_path.read_text(encoding="utf-8"))


def save_token_data(payload: dict[str, Any], path: Path | None = None) -> Path:
    token_path = path or default_token_store_path()
    _save_json_secure(token_path, payload)
    return token_path


def save_pending_oauth_data(payload: dict[str, Any], path: Path | None = None) -> Path:
    pending_path = path or default_pending_oauth_path()
    _save_json_secure(pending_path, payload)
    return pending_path


def load_pending_oauth_data(path: Path | None = None) -> dict[str, Any]:
    pending_path = path or default_pending_oauth_path()
    if not pending_path.exists():
        raise FileNotFoundError(
            f"pending OAuth file not found at {pending_path}; start a login flow first"
        )
    return json.loads(pending_path.read_text(encoding="utf-8"))


def delete_pending_oauth_data(path: Path | None = None) -> None:
    pending_path = path or default_pending_oauth_path()
    if pending_path.exists():
        pending_path.unlink()


def register_client(metadata: dict[str, Any], redirect_uri: str) -> dict[str, Any]:
    registration_endpoint = metadata.get("registration_endpoint")
    if not registration_endpoint:
        raise RuntimeError("authorization server does not advertise dynamic client registration")

    registration_request = {
        "client_name": "notioncli",
        "client_uri": "https://github.com/donutdaniel/notioncli",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    }
    _, _, response = _http_json(
        registration_endpoint,
        method="POST",
        json_body=registration_request,
    )
    if not isinstance(response, dict) or "client_id" not in response:
        raise RuntimeError("dynamic client registration did not return a client_id")
    return response


class _OAuthCallbackHandler(BaseHTTPRequestHandler):
    server_version = "notioncli-mcp/1.0"
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        params = urllib.parse.parse_qs(parsed.query)
        self.server.oauth_result = {  # type: ignore[attr-defined]
            key: values[0] for key, values in params.items() if values
        }
        body = b"You can close this window and return to the terminal.\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
        return


def _exchange_oauth_code(
    *,
    code: str,
    state: str,
    callback_state: str,
    client_id: str,
    client_secret: str | None,
    redirect_uri: str,
    code_verifier: str,
    token_endpoint: str,
    token_path: Path | None = None,
) -> Path:
    if callback_state != state:
        raise RuntimeError("invalid state returned from OAuth callback")

    token_request = {
        "grant_type": "authorization_code",
        "code": code,
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "code_verifier": code_verifier,
    }
    if client_secret:
        token_request["client_secret"] = client_secret

    _, _, tokens = _http_json(
        token_endpoint,
        method="POST",
        headers={"User-Agent": "notioncli-mcp-bench/1.0"},
        form_body=token_request,
    )
    if not isinstance(tokens, dict) or "access_token" not in tokens:
        raise RuntimeError("token exchange did not return an access token")

    now = int(time.time())
    expires_at = now + int(tokens.get("expires_in", 3600)) - 60
    stored = {
        "server_url": DEFAULT_MCP_SERVER_URL,
        "client_id": client_id,
        "client_secret": client_secret,
        "redirect_uri": redirect_uri,
        "access_token": tokens["access_token"],
        "refresh_token": tokens.get("refresh_token"),
        "expires_at": expires_at,
        "token_endpoint": token_endpoint,
        "protocol_version": DEFAULT_PROTOCOL_VERSION,
    }
    return save_token_data(stored, token_path)


def start_oauth_login(
    *,
    server_url: str = DEFAULT_MCP_SERVER_URL,
    pending_path: Path | None = None,
    open_browser: bool = True,
) -> str:
    metadata = discover_oauth_metadata(server_url)
    loopback = HTTPServer(("127.0.0.1", 0), _OAuthCallbackHandler)
    redirect_uri = f"http://127.0.0.1:{loopback.server_address[1]}/callback"

    client = register_client(metadata, redirect_uri)
    client_id = client["client_id"]
    client_secret = client.get("client_secret")
    state = secrets.token_urlsafe(24)
    code_verifier = generate_code_verifier()
    code_challenge = generate_code_challenge(code_verifier)

    query = {
        "response_type": "code",
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "state": state,
        "code_challenge": code_challenge,
        "code_challenge_method": "S256",
    }
    authorization_url = metadata["authorization_endpoint"] + "?" + urllib.parse.urlencode(query)

    save_pending_oauth_data(
        {
            "server_url": server_url,
            "client_id": client_id,
            "client_secret": client_secret,
            "redirect_uri": redirect_uri,
            "state": state,
            "code_verifier": code_verifier,
            "token_endpoint": metadata["token_endpoint"],
        },
        pending_path,
    )

    if open_browser:
        webbrowser.open(authorization_url)
        print(f"Opened browser for Notion MCP login.\nIf it did not open, visit:\n{authorization_url}")
    else:
        print("Visit this URL to authorize Notion MCP:")
        print(authorization_url)

    return authorization_url


def complete_oauth_login(
    *,
    callback_url: str,
    token_path: Path | None = None,
    pending_path: Path | None = None,
) -> Path:
    pending = load_pending_oauth_data(pending_path)
    parsed_callback = urllib.parse.urlparse(callback_url)
    callback_params = {
        key: values[0]
        for key, values in urllib.parse.parse_qs(parsed_callback.query).items()
        if values
    }
    if error_code := callback_params.get("error"):
        description = callback_params.get("error_description", "unknown error")
        raise RuntimeError(f"OAuth error: {error_code} - {description}")
    code = callback_params.get("code")
    if not code:
        raise RuntimeError("missing authorization code in OAuth callback")

    path = _exchange_oauth_code(
        code=code,
        state=pending["state"],
        callback_state=callback_params.get("state", ""),
        client_id=pending["client_id"],
        client_secret=pending.get("client_secret"),
        redirect_uri=pending["redirect_uri"],
        code_verifier=pending["code_verifier"],
        token_endpoint=pending["token_endpoint"],
        token_path=token_path,
    )
    delete_pending_oauth_data(pending_path)
    return path


def perform_oauth_login(
    *,
    server_url: str = DEFAULT_MCP_SERVER_URL,
    token_path: Path | None = None,
    pending_path: Path | None = None,
    open_browser: bool = True,
    timeout_seconds: int = 300,
    manual_callback_url: str | None = None,
) -> Path:
    if manual_callback_url:
        return complete_oauth_login(
            callback_url=manual_callback_url,
            token_path=token_path,
            pending_path=pending_path,
        )

    metadata = discover_oauth_metadata(server_url)
    loopback = HTTPServer(("127.0.0.1", 0), _OAuthCallbackHandler)
    redirect_uri = f"http://127.0.0.1:{loopback.server_address[1]}/callback"

    client = register_client(metadata, redirect_uri)
    client_id = client["client_id"]
    client_secret = client.get("client_secret")
    state = secrets.token_urlsafe(24)
    code_verifier = generate_code_verifier()
    code_challenge = generate_code_challenge(code_verifier)

    query = {
        "response_type": "code",
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "state": state,
        "code_challenge": code_challenge,
        "code_challenge_method": "S256",
    }
    authorization_url = metadata["authorization_endpoint"] + "?" + urllib.parse.urlencode(query)

    save_pending_oauth_data(
        {
            "server_url": server_url,
            "client_id": client_id,
            "client_secret": client_secret,
            "redirect_uri": redirect_uri,
            "state": state,
            "code_verifier": code_verifier,
            "token_endpoint": metadata["token_endpoint"],
        },
        pending_path,
    )

    if open_browser:
        webbrowser.open(authorization_url)
        print(f"Opened browser for Notion MCP login.\nIf it did not open, visit:\n{authorization_url}")
    else:
        print("Visit this URL to authorize Notion MCP:")
        print(authorization_url)

    loopback.timeout = 1.0
    deadline = time.time() + timeout_seconds
    while time.time() < deadline:
        loopback.handle_request()
        callback_params = getattr(loopback, "oauth_result", None)
        if callback_params:
            break
    else:
        raise TimeoutError("timed out waiting for OAuth callback")

    callback_params = callback_params or {}
    if error_code := callback_params.get("error"):
        description = callback_params.get("error_description", "unknown error")
        raise RuntimeError(f"OAuth error: {error_code} - {description}")
    code = callback_params.get("code")
    if not code:
        raise RuntimeError("missing authorization code in OAuth callback")

    path = _exchange_oauth_code(
        code=code,
        state=state,
        callback_state=callback_params.get("state", ""),
        client_id=client_id,
        client_secret=client_secret,
        redirect_uri=redirect_uri,
        code_verifier=code_verifier,
        token_endpoint=metadata["token_endpoint"],
        token_path=token_path,
    )
    delete_pending_oauth_data(pending_path)
    return path


def refresh_token_if_needed(token_data: dict[str, Any], path: Path | None = None) -> dict[str, Any]:
    expires_at = int(token_data.get("expires_at", 0))
    refresh_token = token_data.get("refresh_token")
    if expires_at > int(time.time()) and token_data.get("access_token"):
        return token_data
    if not refresh_token:
        raise RuntimeError("MCP access token expired and no refresh token is stored")

    token_request = {
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": token_data["client_id"],
    }
    if token_data.get("client_secret"):
        token_request["client_secret"] = token_data["client_secret"]

    _, _, refreshed = _http_json(
        token_data["token_endpoint"],
        method="POST",
        headers={"User-Agent": "notioncli-mcp-bench/1.0"},
        form_body=token_request,
    )
    if not isinstance(refreshed, dict) or "access_token" not in refreshed:
        raise RuntimeError("refresh token exchange did not return an access token")

    token_data["access_token"] = refreshed["access_token"]
    if "refresh_token" in refreshed and refreshed["refresh_token"]:
        token_data["refresh_token"] = refreshed["refresh_token"]
    token_data["expires_at"] = int(time.time()) + int(refreshed.get("expires_in", 3600)) - 60
    save_token_data(token_data, path)
    return token_data


def load_access_token(token_path: Path | None = None) -> tuple[str, dict[str, Any] | None]:
    if direct := os.environ.get("NOTION_MCP_ACCESS_TOKEN"):
        return direct, None
    token_data = refresh_token_if_needed(load_token_data(token_path), token_path)
    return token_data["access_token"], token_data


def _extract_json_rpc_message(response_text: str) -> dict[str, Any] | None:
    text = response_text.strip()
    if not text:
        return None
    if text.startswith("{"):
        return json.loads(text)

    message: dict[str, Any] | None = None
    chunks = [chunk.strip() for chunk in text.split("\n\n") if chunk.strip()]
    for chunk in chunks:
        data_lines = []
        for line in chunk.splitlines():
            if line.startswith("data:"):
                data_lines.append(line[5:].strip())
        if not data_lines:
            continue
        candidate = "\n".join(data_lines)
        if candidate and candidate != "[DONE]":
            try:
                parsed = json.loads(candidate)
            except json.JSONDecodeError:
                continue
            if isinstance(parsed, dict):
                message = parsed
    return message


@dataclass
class NotionMcpClient:
    server_url: str
    access_token: str
    protocol_version: str = DEFAULT_PROTOCOL_VERSION
    session_id: str | None = None
    _next_id: int = 1

    def initialize(self) -> dict[str, Any]:
        payload = {
            "jsonrpc": "2.0",
            "id": self._consume_id(),
            "method": "initialize",
            "params": {
                "protocolVersion": self.protocol_version,
                "capabilities": {},
                "clientInfo": {
                    "name": "notioncli-bench",
                    "version": "0.1.0",
                },
            },
        }
        message = self._post(payload)
        self.notify("notifications/initialized", {})
        return message

    def list_tools(self) -> list[dict[str, Any]]:
        tools: list[dict[str, Any]] = []
        cursor: str | None = None
        while True:
            params: dict[str, Any] = {}
            if cursor:
                params["cursor"] = cursor
            message = self.rpc("tools/list", params)
            result = message.get("result", {})
            tools.extend(result.get("tools", []))
            cursor = result.get("nextCursor")
            if not cursor:
                return tools

    def call_tool(self, name: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
        return self.rpc(
            "tools/call",
            {
                "name": name,
                "arguments": arguments or {},
            },
        )

    def rpc(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        payload = {
            "jsonrpc": "2.0",
            "id": self._consume_id(),
            "method": method,
            "params": params or {},
        }
        message = self._post(payload)
        if "error" in message:
            raise RuntimeError(json.dumps(message["error"], sort_keys=True))
        return message

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        payload = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {},
        }
        self._post(payload, expect_response=False)

    def _consume_id(self) -> int:
        current = self._next_id
        self._next_id += 1
        return current

    def _post(self, payload: dict[str, Any], *, expect_response: bool = True) -> dict[str, Any]:
        headers = {
            "Authorization": f"Bearer {self.access_token}",
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
        }
        if self.session_id:
            headers["Mcp-Session-Id"] = self.session_id

        status, response_headers, text = _http_request(
            self.server_url,
            method="POST",
            headers=headers,
            json_body=payload,
            timeout=60.0,
        )
        if session_id := response_headers.get("Mcp-Session-Id") or response_headers.get("mcp-session-id"):
            self.session_id = session_id
        if not expect_response:
            return {}
        message = _extract_json_rpc_message(text)
        if message is None:
            raise RuntimeError(f"MCP server returned an empty response for payload: {payload}")
        return message
