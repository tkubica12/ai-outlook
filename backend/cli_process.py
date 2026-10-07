from __future__ import annotations

import asyncio
import os
from collections.abc import Sequence

from backend.mcp import McpError


async def _stop_process(process: asyncio.subprocess.Process) -> None:
    if process.returncode is not None:
        return
    if os.name == "nt":
        # Copilot can own MCP child processes; killing only its parent leaks them.
        killer = await asyncio.create_subprocess_exec(
            "taskkill", "/PID", str(process.pid), "/T", "/F",
            stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL,
        )
        await killer.wait()
    if process.returncode is None:
        try:
            process.kill()
        except ProcessLookupError:
            pass
    await process.wait()


async def run_cli(
    args: Sequence[str], *, timeout: float, timeout_message: str, cwd: str | None = None
) -> str:
    try:
        process = await asyncio.create_subprocess_exec(
            *args, cwd=cwd,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
        )
    except OSError as error:
        raise McpError("Could not start Copilot CLI; check its installation and runtime configuration.") from error
    try:
        stdout, _stderr = await asyncio.wait_for(process.communicate(), timeout=timeout)
    except TimeoutError:
        await _stop_process(process)
        raise McpError(timeout_message) from None
    except asyncio.CancelledError:
        await _stop_process(process)
        raise
    if process.returncode != 0:
        # CLI stderr may contain retrieved records or credential-bearing URLs.
        raise McpError(
            f"Copilot CLI exited with code {process.returncode}. "
            "Check Copilot sign-in and the required MCP connection, then retry."
        )
    return stdout.decode(errors="replace")
