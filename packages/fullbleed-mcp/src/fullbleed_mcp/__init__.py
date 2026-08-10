"""First-party MCP entrypoint for Fullbleed PDF Engine."""

from __future__ import annotations

from fullbleed_cli.mcp import FullbleedMcpServer, main, serve_stdio


__all__ = ["FullbleedMcpServer", "main", "serve_stdio"]
