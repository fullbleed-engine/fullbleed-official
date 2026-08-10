from __future__ import annotations

from pathlib import Path

import pytest

from fullbleed_cli.mcp import FullbleedMcpServer


def _request(request_id: int, method: str, params=None):
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params or {},
    }


def test_mcp_initialization_and_tool_discovery(tmp_path: Path) -> None:
    server = FullbleedMcpServer(tmp_path)
    initialized = server.handle_message(
        _request(1, "initialize", {"protocolVersion": "2025-11-25"})
    )
    assert initialized["result"]["protocolVersion"] == "2025-11-25"
    assert initialized["result"]["capabilities"] == {
        "tools": {"listChanged": False}
    }
    assert "deterministic print document" in initialized["result"]["instructions"]

    listed = server.handle_message(_request(2, "tools/list"))
    tools = listed["result"]["tools"]
    assert {tool["name"] for tool in tools} == {
        "fullbleed_capabilities",
        "fullbleed_agent_contract",
        "fullbleed_create_project",
        "fullbleed_render",
        "fullbleed_render_preview",
        "fullbleed_verify",
        "fullbleed_inspect",
        "fullbleed_assets",
        "fullbleed_compile",
        "fullbleed_render_compiled",
        "fullbleed_compile_vdp",
    }
    assert all(tool["inputSchema"]["type"] == "object" for tool in tools)
    assert all(tool["outputSchema"].get("anyOf") for tool in tools)
    compile_tool = next(tool for tool in tools if tool["name"] == "fullbleed_compile")
    assert compile_tool["annotations"]["readOnlyHint"] is False
    assert "compile_id" in compile_tool["outputSchema"]["anyOf"][0]["properties"]


def test_mcp_notifications_and_protocol_errors(tmp_path: Path) -> None:
    server = FullbleedMcpServer(tmp_path)
    assert (
        server.handle_message(
            {"jsonrpc": "2.0", "method": "notifications/initialized"}
        )
        is None
    )
    missing = server.handle_message(_request(1, "does/not/exist"))
    assert missing["error"]["code"] == -32601
    invalid = server.handle_message({"jsonrpc": "2.0", "id": 2})
    assert invalid["error"]["code"] == -32600


def test_mcp_confines_user_paths_to_workspace(tmp_path: Path) -> None:
    server = FullbleedMcpServer(tmp_path)
    outside = tmp_path.parent / "outside.pdf"
    with pytest.raises(ValueError, match="escapes"):
        server._path(str(outside))

    nested = server._output_path("output/document.pdf")
    assert nested.parent.is_dir()
    assert tmp_path.resolve() in nested.parents


def test_mcp_tool_failures_are_model_visible(tmp_path: Path) -> None:
    server = FullbleedMcpServer(tmp_path)
    response = server.handle_message(
        _request(
            4,
            "tools/call",
            {
                "name": "fullbleed_render",
                "arguments": {"output_path": "output/missing-source.pdf"},
            },
        )
    )
    result = response["result"]
    assert result["isError"] is True
    assert result["structuredContent"]["code"] == "MCP_TOOL_ERROR"
    assert "exactly one" in result["structuredContent"]["message"]


def test_mcp_capabilities_tool_returns_structured_runtime_contract(tmp_path: Path) -> None:
    server = FullbleedMcpServer(tmp_path)
    response = server.handle_message(
        _request(
            5,
            "tools/call",
            {"name": "fullbleed_capabilities", "arguments": {}},
        )
    )
    result = response["result"]
    assert result.get("isError") is None
    assert result["structuredContent"]["schema"] == "fullbleed.capabilities.v1"
    assert (
        result["structuredContent"]["engine"]["compiled_document"] is True
    )
