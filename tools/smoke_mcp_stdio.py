#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Cold-process smoke test for the installed fullbleed-mcp stdio server."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import uuid


def _request(request_id: int, method: str, params: dict | None = None) -> dict:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params or {},
    }


def run(executable: str | None = None, *, container_image: str | None = None) -> dict:
    with tempfile.TemporaryDirectory(prefix="fullbleed-mcp-smoke-") as raw:
        container_name = None
        command = (
            [executable, "--root", raw]
            if executable
            else [sys.executable, "-m", "fullbleed_mcp", "--root", raw]
        )
        if container_image:
            container_name = f"fullbleed-mcp-smoke-{uuid.uuid4().hex}"
            command = [
                "docker", "run", "--rm", "-i", "--name", container_name,
                "--network", "none", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--read-only",
                "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m",
                "--mount", f"type=bind,source={Path(raw).resolve()},target=/workspace",
            ]
            if os.name == "posix":
                # Let the container write only this temporary host workspace.
                command.extend(["--user", f"{os.getuid()}:{os.getgid()}"])
            command.extend([container_image, "--root", "/workspace"])
        requests = [
            _request(
                1,
                "initialize",
                {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "fullbleed-smoke", "version": "1"},
                },
            ),
            _request(2, "tools/list"),
            _request(
                3,
                "tools/call",
                {"name": "fullbleed_capabilities", "arguments": {}},
            ),
            _request(
                4,
                "tools/call",
                {
                    "name": "fullbleed_render_preview",
                    "arguments": {
                        "html": "<main><h1>MCP Preview</h1><p>MCP-SMOKE-001</p></main>",
                        "css": "@page{size:240pt 180pt;margin:18pt}body{font:11pt Helvetica,sans-serif}",
                        "output_dir": "preview",
                    },
                },
            ),
            _request(
                5,
                "tools/call",
                {
                    "name": "fullbleed_inspect",
                    "arguments": {"path": "preview/preview.pdf"},
                },
            ),
            _request(
                6,
                "tools/call",
                {
                    "name": "fullbleed_verify",
                    "arguments": {
                        "html": "<main><p>MCP-VERIFY-001</p></main>",
                        "css": "@page{size:240pt 180pt;margin:18pt}body{font:11pt Helvetica,sans-serif}",
                        "fail_on": ["missing-glyphs"],
                    },
                },
            ),
            _request(
                7,
                "tools/call",
                {
                    "name": "fullbleed_compile_vdp",
                    "arguments": {
                        "html": "<main><h1>Statement {{record_id}}</h1></main>",
                        "css": "@page{size:240pt 180pt;margin:18pt}body{font:11pt Helvetica,sans-serif}",
                        "bindings": {
                            "record_id": ["MCP-VDP-001", "MCP-VDP-002", "MCP-VDP-003"]
                        },
                        "mode": "fixed_bindings",
                        "output_path": "output/vdp.pdf",
                    },
                },
            ),
        ]
        if container_image:
            requests.append(_request(8, "tools/call", {
                "name": "fullbleed_inspect", "arguments": {"path": "/etc/passwd"},
            }))
        process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, encoding="utf-8",
        )
        assert process.stdin is not None
        assert process.stdout is not None
        assert process.stderr is not None
        # Bound each cold-process session, including a stalled stdout read.
        timer = threading.Timer(90, process.kill)
        timer.daemon = True
        timer.start()
        responses = []
        try:
            for request in requests:
                process.stdin.write(json.dumps(request) + "\n")
                process.stdin.flush()
                line = process.stdout.readline()
                if not line:
                    raise AssertionError("MCP server closed stdout before responding")
                responses.append(json.loads(line))
                if request["method"] == "initialize":
                    process.stdin.write(json.dumps({
                        "jsonrpc": "2.0", "method": "notifications/initialized",
                    }) + "\n")
                    process.stdin.flush()
            process.stdin.close()
            return_code = process.wait(timeout=15)
            stderr = process.stderr.read()
        finally:
            timer.cancel()
            if process.poll() is None:
                process.kill()
                process.wait(timeout=15)
            if container_name:
                # --rm handles normal exits; also clean up our container on timeout.
                subprocess.run(
                    ["docker", "rm", "--force", container_name],
                    capture_output=True, timeout=20, check=False,
                )
        if len(responses) != len(requests):
            raise AssertionError(f"MCP response count mismatch: {len(responses)}; {stderr}")
        if [response.get("id") for response in responses] != [request["id"] for request in requests]:
            raise AssertionError("MCP response IDs do not match the requests")
        artifacts = {
            "preview_pdf": (Path(raw) / "preview" / "preview.pdf").is_file(),
            "preview_png": any((Path(raw) / "preview" / "pages").glob("*.png")),
            "vdp_pdf": (Path(raw) / "output" / "vdp.pdf").is_file(),
        }
    if return_code != 0:
        raise AssertionError(f"MCP server exited {return_code}: {stderr}")
    (
        initialized,
        listed,
        capability_call,
        preview_call,
        inspect_call,
        verify_call,
        vdp_call,
    ) = responses[:7]
    if initialized["result"]["protocolVersion"] != "2025-11-25":
        raise AssertionError("MCP protocol negotiation failed")
    tool_names = {tool["name"] for tool in listed["result"]["tools"]}
    required = {
        "fullbleed_capabilities",
        "fullbleed_create_project",
        "fullbleed_render",
        "fullbleed_render_preview",
        "fullbleed_inspect",
        "fullbleed_verify",
        "fullbleed_compile_vdp",
    }
    if not required.issubset(tool_names):
        raise AssertionError(f"MCP tools missing: {sorted(required - tool_names)}")
    capability = capability_call["result"]["structuredContent"]
    if capability.get("schema") != "fullbleed.capabilities.v1":
        raise AssertionError("MCP capabilities tool returned the wrong schema")
    for label, response in (
        ("preview", preview_call),
        ("inspect", inspect_call),
        ("verify", verify_call),
        ("VDP", vdp_call),
    ):
        result = response["result"]
        if result.get("isError") or result["structuredContent"].get("ok") is not True:
            raise AssertionError(f"MCP {label} call failed: {result}")
    if inspect_call["result"]["structuredContent"].get("page_count") != 1:
        raise AssertionError("MCP inspect did not observe the preview page")
    vdp = vdp_call["result"]["structuredContent"]
    if vdp.get("record_count") != 3 or vdp.get("page_count") != 3:
        raise AssertionError("MCP compiled VDP did not produce three records/pages")
    if not all(artifacts.values()):
        raise AssertionError(f"MCP artifact set is incomplete: {artifacts}")
    result = {
        "schema": "fullbleed.mcp_stdio_smoke.v1",
        "ok": True,
        "protocol_version": initialized["result"]["protocolVersion"],
        "tool_count": len(tool_names),
        "runtime_commands": capability.get("commands", []),
        "artifacts": artifacts,
        "vdp": {
            "record_count": vdp["record_count"],
            "page_count": vdp["page_count"],
            "bytes_written": vdp["bytes_written"],
        },
    }
    if container_image:
        denied = responses[7].get("result", {})
        if not denied.get("isError") or "escapes MCP workspace root" not in json.dumps(denied):
            raise AssertionError("MCP container did not reject a path outside its workspace")
        result["container"] = {
            "image": container_image,
            "network": "none",
            "read_only_image": True,
            "workspace_escape_rejected": True,
        }
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    transport = parser.add_mutually_exclusive_group()
    transport.add_argument(
        "--executable",
        help="Optional fullbleed-mcp executable; defaults to the current Python module",
    )
    transport.add_argument(
        "--container-image", help="Exercise a built MCP Docker image with a temporary workspace",
    )
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)
    result = run(arguments.executable, container_image=arguments.container_image)
    if arguments.json:
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(
            f"[ok] MCP stdio smoke ({result['tool_count']} tools, "
            f"protocol {result['protocol_version']})\n"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
