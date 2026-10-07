from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

import httpx


class McpError(RuntimeError):
    pass


@dataclass
class McpHttpClient:
    url: str
    headers: dict[str, str] = field(default_factory=dict)
    timeout: float = 30
    _session_id: str | None = None
    _request_id: int = 0

    async def _post(self, method: str, params: dict[str, Any] | None = None) -> Any:
        self._request_id += 1
        headers = {
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
            **self.headers,
        }
        if self._session_id:
            headers["Mcp-Session-Id"] = self._session_id
        payload = {
            "jsonrpc": "2.0",
            "id": self._request_id,
            "method": method,
            "params": params or {},
        }
        async with httpx.AsyncClient(timeout=self.timeout, follow_redirects=True) as client:
            response = await client.post(self.url, headers=headers, json=payload)
        if response.status_code in {401, 403}:
            raise McpError("Authentication required or access was denied")
        response.raise_for_status()
        self._session_id = response.headers.get("Mcp-Session-Id", self._session_id)
        data = self._decode(response)
        if "error" in data:
            message = data["error"].get("message", "MCP request failed")
            raise McpError(message)
        return data.get("result")

    @staticmethod
    def _decode(response: httpx.Response) -> dict[str, Any]:
        content_type = response.headers.get("content-type", "")
        if "text/event-stream" not in content_type:
            return response.json()
        for line in reversed(response.text.splitlines()):
            if line.startswith("data:"):
                return json.loads(line[5:].strip())
        raise McpError("MCP server returned an empty event stream")

    async def initialize(self) -> None:
        if self._session_id:
            return
        await self._post(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "tomlook", "version": "0.2.0"},
            },
        )
        headers = {
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
            **self.headers,
        }
        if self._session_id:
            headers["Mcp-Session-Id"] = self._session_id
        async with httpx.AsyncClient(timeout=self.timeout, follow_redirects=True) as client:
            response = await client.post(
                self.url,
                headers=headers,
                json={"jsonrpc": "2.0", "method": "notifications/initialized"},
            )
        response.raise_for_status()

    async def list_tools(self) -> list[dict[str, Any]]:
        await self.initialize()
        result = await self._post("tools/list")
        return result.get("tools", [])

    async def call_tool(self, name: str, arguments: dict[str, Any]) -> Any:
        await self.initialize()
        result = await self._post("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise McpError(self.result_text(result) or f"MCP tool {name} failed")
        if result.get("structuredContent") is not None:
            return result["structuredContent"]
        text = self.result_text(result)
        try:
            return json.loads(text)
        except (TypeError, json.JSONDecodeError):
            return text

    @staticmethod
    def result_text(result: dict[str, Any]) -> str:
        return "\n".join(
            item.get("text", "") for item in result.get("content", []) if item.get("type") == "text"
        )
