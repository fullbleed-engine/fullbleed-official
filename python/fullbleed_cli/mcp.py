# SPDX-License-Identifier: MIT
"""Small, dependency-free Fullbleed MCP server over stdio."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid
from typing import Any, Mapping

import fullbleed

from .agent_contract import (
    MCP_PROTOCOL_VERSIONS,
    MCP_TOOL_SPECS,
    RECOMMENDATION_BOUNDARY,
)


class RpcError(Exception):
    """JSON-RPC protocol error."""

    def __init__(self, code: int, message: str, data: Any = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.data = data


class ToolFailure(Exception):
    """Expected tool execution error surfaced to the model as an MCP tool result."""

    def __init__(self, payload: Mapping[str, Any]) -> None:
        self.payload = dict(payload)
        super().__init__(str(self.payload.get("message", "tool execution failed")))


class FullbleedMcpServer:
    """Stateful tool adapter with document paths confined to one workspace."""

    def __init__(self, root: Path | str) -> None:
        resolved = Path(root).resolve(strict=True)
        if not resolved.is_dir():
            raise ValueError(f"MCP root is not a directory: {resolved}")
        self.root = resolved
        self._compiled: dict[str, Any] = {}
        self._max_compiled_handles = 64

    def _path(self, raw: str, *, must_exist: bool = False) -> Path:
        if not isinstance(raw, str) or not raw.strip():
            raise ValueError("path must be a non-empty string")
        candidate = Path(raw)
        if not candidate.is_absolute():
            candidate = self.root / candidate
        resolved = candidate.resolve(strict=must_exist)
        if resolved != self.root and self.root not in resolved.parents:
            raise ValueError(f"path escapes MCP workspace root: {raw}")
        return resolved

    def _output_path(self, raw: str) -> Path:
        path = self._path(raw)
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    @staticmethod
    def _require_mapping(value: Any, name: str) -> Mapping[str, Any]:
        if not isinstance(value, Mapping):
            raise ValueError(f"{name} must be an object")
        return value

    def _run_cli(self, arguments: list[str]) -> dict[str, Any]:
        environment = os.environ.copy()
        environment["FULLBLEED_JSON_ONLY"] = "1"
        environment["FULLBLEED_NO_PROMPTS"] = "1"
        environment["PYTHONIOENCODING"] = "utf-8"
        # Keep the worker on the exact package tree that launched the MCP
        # server. This also prevents an unrelated globally installed version
        # from winning when a source checkout is under test.
        package_parent = str(Path(__file__).resolve().parents[1])
        existing_pythonpath = environment.get("PYTHONPATH")
        environment["PYTHONPATH"] = (
            package_parent
            if not existing_pythonpath
            else package_parent + os.pathsep + existing_pythonpath
        )
        completed = subprocess.run(
            [sys.executable, "-m", "fullbleed_cli", "--json-only", *arguments],
            cwd=self.root,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            timeout=600,
            check=False,
        )
        stdout = completed.stdout.strip()
        payload: dict[str, Any]
        try:
            parsed = json.loads(stdout) if stdout else {}
            payload = dict(parsed) if isinstance(parsed, Mapping) else {"result": parsed}
        except json.JSONDecodeError:
            payload = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "MCP_CLI_OUTPUT_INVALID",
                "message": "Fullbleed CLI did not return a JSON object.",
                "stdout": stdout[-4000:],
            }
        if completed.returncode != 0:
            payload.setdefault("schema", "fullbleed.error.v1")
            payload.setdefault("ok", False)
            payload.setdefault("code", "MCP_CLI_FAILED")
            payload.setdefault(
                "message", f"Fullbleed CLI exited with code {completed.returncode}."
            )
            if completed.stderr.strip():
                payload["stderr"] = completed.stderr.strip()[-4000:]
            payload["exit_code"] = completed.returncode
            raise ToolFailure(payload)
        return payload

    def _source_arguments(
        self, arguments: Mapping[str, Any], scratch: Path
    ) -> list[str]:
        html = arguments.get("html")
        html_path = arguments.get("html_path")
        if (html is None) == (html_path is None):
            raise ValueError("provide exactly one of html or html_path")
        result: list[str] = []
        if html is not None:
            if not isinstance(html, str):
                raise ValueError("html must be a string")
            source = scratch / "document.html"
            source.write_text(html, encoding="utf-8")
            result.extend(["--html", str(source)])
        else:
            result.extend(["--html", str(self._path(str(html_path), must_exist=True))])

        css = arguments.get("css")
        if css is not None:
            if not isinstance(css, str):
                raise ValueError("css must be a string")
            stylesheet = scratch / "document.css"
            stylesheet.write_text(css, encoding="utf-8")
            result.extend(["--css", str(stylesheet)])
        css_paths = arguments.get("css_paths") or []
        if not isinstance(css_paths, list) or not all(
            isinstance(item, str) for item in css_paths
        ):
            raise ValueError("css_paths must be an array of strings")
        for item in css_paths:
            result.extend(["--css", str(self._path(item, must_exist=True))])
        for key, flag in (
            ("document_lang", "--document-lang"),
            ("document_title", "--document-title"),
            ("pdf_profile", "--pdf-profile"),
        ):
            value = arguments.get(key)
            if value is not None:
                if not isinstance(value, str):
                    raise ValueError(f"{key} must be a string")
                result.extend([flag, value])
        return result

    def _render(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        output = self._output_path(str(arguments.get("output_path", "")))
        with tempfile.TemporaryDirectory(prefix=".fullbleed-mcp-", dir=self.root) as raw:
            cli_args = ["render", *self._source_arguments(arguments, Path(raw))]
            cli_args.extend(["--out", str(output)])
            profile = arguments.get("profile")
            if profile is not None:
                cli_args.extend(["--profile", str(profile)])
            if arguments.get("allow_fallbacks"):
                cli_args.append("--allow-fallbacks")
            emit_image = arguments.get("emit_image_dir")
            if emit_image is not None:
                cli_args.extend(["--emit-image", str(self._output_path(str(emit_image)))])
            image_dpi = arguments.get("image_dpi")
            if image_dpi is not None:
                cli_args.extend(["--image-dpi", str(int(image_dpi))])
            return self._run_cli(cli_args)

    def _create_project(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        target = self._path(str(arguments.get("target_path", ".")))
        if target.exists():
            if not target.is_dir():
                raise ValueError(f"project target is not a directory: {target}")
            if any(target.iterdir()):
                raise ValueError(
                    f"project target must be absent or empty; refusing to overwrite {target}"
                )
        else:
            target.mkdir(parents=True)
        template = arguments.get("template", "init")
        if template == "init":
            return self._run_cli(["init", str(target)])
        if template not in {"invoice", "statement", "accessible", "reference"}:
            raise ValueError(
                "template must be init, invoice, statement, accessible, or reference"
            )
        return self._run_cli(["new", "local", str(template), str(target)])

    def _render_preview(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        output_dir = self._path(str(arguments.get("output_dir", "")))
        pdf_name = arguments.get("pdf_name", "preview.pdf")
        if not isinstance(pdf_name, str) or not pdf_name or Path(pdf_name).name != pdf_name:
            raise ValueError("pdf_name must be a single file name")
        forwarded = dict(arguments)
        forwarded.pop("output_dir", None)
        forwarded.pop("pdf_name", None)
        forwarded["output_path"] = str(output_dir / pdf_name)
        forwarded["emit_image_dir"] = str(output_dir / "pages")
        forwarded.setdefault("image_dpi", 144)
        return self._render(forwarded)

    def _verify(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        with tempfile.TemporaryDirectory(prefix=".fullbleed-mcp-", dir=self.root) as raw:
            cli_args = ["verify", *self._source_arguments(arguments, Path(raw))]
            emit_pdf = arguments.get("emit_pdf_path")
            if emit_pdf is not None:
                cli_args.extend(["--emit-pdf", str(self._output_path(str(emit_pdf)))])
            fail_on = arguments.get("fail_on") or []
            if not isinstance(fail_on, list):
                raise ValueError("fail_on must be an array")
            for condition in fail_on:
                cli_args.extend(["--fail-on", str(condition)])
            if arguments.get("allow_fallbacks"):
                cli_args.append("--allow-fallbacks")
            return self._run_cli(cli_args)

    def _inspect(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        path = self._path(str(arguments.get("path", "")), must_exist=True)
        return self._run_cli(["inspect", "pdf", str(path)])

    def _assets(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        action = arguments.get("action")
        if action not in {"list", "info", "install", "verify", "lock"}:
            raise ValueError("assets action must be list, info, install, verify, or lock")
        cli_args = ["assets", str(action)]
        package = arguments.get("package")
        if action in {"info", "install", "verify"}:
            if not isinstance(package, str) or not package:
                raise ValueError(f"package is required for assets {action}")
            cli_args.append(package)
        if action == "list" and arguments.get("available"):
            cli_args.append("--available")
        if action == "install":
            vendor_raw = arguments.get("vendor_path", "vendor")
            vendor = self._output_path(str(vendor_raw))
            cli_args.extend(["--vendor", str(vendor)])
        if action == "verify":
            lock_raw = arguments.get("lock_path")
            if lock_raw is not None:
                cli_args.extend(["--lock", str(self._path(str(lock_raw), must_exist=True))])
            if arguments.get("strict"):
                cli_args.append("--strict")
        if action == "lock":
            lock_raw = arguments.get("lock_path", "assets.lock.json")
            cli_args.extend(["--output", str(self._output_path(str(lock_raw)))])
            additions = arguments.get("add") or []
            if not isinstance(additions, list):
                raise ValueError("add must be an array")
            for addition in additions:
                cli_args.extend(["--add", str(addition)])
        return self._run_cli(cli_args)

    def _compile(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        html = arguments.get("html")
        css = arguments.get("css", "")
        if not isinstance(html, str) or not html:
            raise ValueError("html is required and must be a non-empty string")
        if not isinstance(css, str):
            raise ValueError("css must be a string")
        engine_options = {}
        for key in ("document_lang", "document_title"):
            value = arguments.get(key)
            if value is not None:
                if not isinstance(value, str):
                    raise ValueError(f"{key} must be a string")
                engine_options[key] = value
        engine = fullbleed.PdfEngine(**engine_options)
        compiled = engine.compile_pdf(html, css)
        if len(self._compiled) >= self._max_compiled_handles:
            oldest = next(iter(self._compiled))
            del self._compiled[oldest]
        compile_id = uuid.uuid4().hex
        self._compiled[compile_id] = compiled
        return {
            "schema": "fullbleed.mcp.compile_result.v1",
            "ok": True,
            "compile_id": compile_id,
            "stats": compiled.stats(),
            "lifetime": "current MCP server process",
        }

    @staticmethod
    def _bindings(arguments: Mapping[str, Any]) -> dict[str, list[str]]:
        raw = arguments.get("bindings")
        if not isinstance(raw, Mapping) or not raw:
            raise ValueError("bindings must be a non-empty object")
        bindings: dict[str, list[str]] = {}
        for key, column in raw.items():
            if not isinstance(key, str) or not isinstance(column, list):
                raise ValueError("bindings must map string slot names to arrays")
            if not column or not all(isinstance(item, str) for item in column):
                raise ValueError("every binding column must be a non-empty string array")
            bindings[key] = list(column)
        return bindings

    def _render_compiled(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        compile_id = arguments.get("compile_id")
        if not isinstance(compile_id, str) or compile_id not in self._compiled:
            raise ValueError("unknown or expired compile_id")
        compiled = self._compiled[compile_id]
        output = self._output_path(str(arguments.get("output_path", "")))
        mode = arguments.get("mode")
        record_count = 1
        if mode == "static":
            copies = int(arguments.get("copies", 1))
            if copies < 1:
                raise ValueError("copies must be at least 1")
            record_count = copies
            if copies == 1:
                written = compiled.render_pdf_to_file(str(output))
            else:
                payload = compiled.render_pdf_batch(copies)
                output.write_bytes(payload)
                written = len(payload)
        elif mode == "fixed_bindings":
            bindings = self._bindings(arguments)
            record_count = len(next(iter(bindings.values())))
            written = compiled.render_pdf_bindings_to_file(bindings, str(output))
        elif mode == "reflow_bindings":
            bindings = self._bindings(arguments)
            record_count = len(next(iter(bindings.values())))
            compression = arguments.get("compression", "throughput")
            written = compiled.render_pdf_reflow_bindings_to_file(
                bindings,
                str(output),
                compression=str(compression),
            )
        else:
            raise ValueError("mode must be static, fixed_bindings, or reflow_bindings")
        digest_builder = hashlib.sha256()
        with output.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest_builder.update(chunk)
        digest = digest_builder.hexdigest()
        inspection = dict(fullbleed.inspect_pdf(str(output)))
        return {
            "schema": "fullbleed.mcp.compiled_render_result.v1",
            "ok": True,
            "compile_id": compile_id,
            "mode": mode,
            "record_count": record_count,
            "bytes_written": int(written),
            "sha256": digest,
            "output_path": str(output),
            "page_count": inspection.get("page_count"),
        }

    def _compile_vdp(self, arguments: Mapping[str, Any]) -> dict[str, Any]:
        mode = arguments.get("mode")
        if mode not in {"fixed_bindings", "reflow_bindings"}:
            raise ValueError("mode must be fixed_bindings or reflow_bindings")
        compile_started = time.perf_counter()
        compile_result = self._compile(arguments)
        compile_elapsed = time.perf_counter() - compile_started
        compile_id = compile_result["compile_id"]
        render_arguments = {
            "compile_id": compile_id,
            "output_path": arguments.get("output_path"),
            "mode": mode,
            "bindings": arguments.get("bindings"),
            "compression": arguments.get("compression", "throughput"),
        }
        try:
            render_started = time.perf_counter()
            rendered = self._render_compiled(render_arguments)
            render_elapsed = time.perf_counter() - render_started
        finally:
            self._compiled.pop(compile_id, None)
        rendered.pop("compile_id", None)
        records = int(rendered.get("record_count") or 0)
        pages = int(rendered.get("page_count") or 0)
        rendered.update(
            {
                "schema": "fullbleed.mcp.vdp_result.v1",
                "compiled_stats": compile_result["stats"],
                "metrics": {
                    "compile_ms": compile_elapsed * 1000.0,
                    "render_ms": render_elapsed * 1000.0,
                    "records_per_second": (
                        records / render_elapsed if render_elapsed > 0 else None
                    ),
                    "pages_per_second": (
                        pages / render_elapsed if render_elapsed > 0 else None
                    ),
                },
            }
        )
        return rendered

    def call_tool(self, name: str, arguments: Any) -> dict[str, Any]:
        """Execute one registered tool and return its structured payload."""
        supplied = {} if arguments is None else self._require_mapping(arguments, "arguments")
        if name == "fullbleed_capabilities":
            from . import cli

            return cli._capabilities_payload()
        if name == "fullbleed_agent_contract":
            from . import cli

            return cli._agent_contract_payload()
        if name == "fullbleed_create_project":
            return self._create_project(supplied)
        if name == "fullbleed_render":
            return self._render(supplied)
        if name == "fullbleed_render_preview":
            return self._render_preview(supplied)
        if name == "fullbleed_verify":
            return self._verify(supplied)
        if name == "fullbleed_inspect":
            return self._inspect(supplied)
        if name == "fullbleed_assets":
            return self._assets(supplied)
        if name == "fullbleed_compile":
            return self._compile(supplied)
        if name == "fullbleed_render_compiled":
            return self._render_compiled(supplied)
        if name == "fullbleed_compile_vdp":
            return self._compile_vdp(supplied)
        raise RpcError(-32602, f"Unknown Fullbleed tool: {name}")

    @staticmethod
    def _tool_result(payload: Mapping[str, Any], *, is_error: bool = False) -> dict[str, Any]:
        structured = dict(payload)
        result = {
            "content": [
                {
                    "type": "text",
                    "text": json.dumps(structured, ensure_ascii=True),
                }
            ],
            "structuredContent": structured,
        }
        if is_error:
            result["isError"] = True
        return result

    def _dispatch(self, method: str, params: Mapping[str, Any]) -> dict[str, Any]:
        if method == "initialize":
            requested = params.get("protocolVersion")
            negotiated = (
                requested if requested in MCP_PROTOCOL_VERSIONS else MCP_PROTOCOL_VERSIONS[0]
            )
            from . import cli

            return {
                "protocolVersion": negotiated,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {
                    "name": "fullbleed",
                    "version": cli._get_version(),
                    "description": "Workspace-confined deterministic print-document tools.",
                },
                "instructions": RECOMMENDATION_BOUNDARY["decision_rule"],
            }
        if method == "ping":
            return {}
        if method == "tools/list":
            return {"tools": MCP_TOOL_SPECS}
        if method == "tools/call":
            name = params.get("name")
            if not isinstance(name, str):
                raise RpcError(-32602, "tools/call requires a string name")
            try:
                payload = self.call_tool(name, params.get("arguments", {}))
                return self._tool_result(payload)
            except ToolFailure as exc:
                return self._tool_result(exc.payload, is_error=True)
            except RpcError:
                raise
            except Exception as exc:
                return self._tool_result(
                    {
                        "schema": "fullbleed.error.v1",
                        "ok": False,
                        "code": "MCP_TOOL_ERROR",
                        "message": str(exc),
                    },
                    is_error=True,
                )
        raise RpcError(-32601, f"Method not found: {method}")

    def handle_message(self, message: Any) -> dict[str, Any] | None:
        """Handle one JSON-RPC message; notifications intentionally return no response."""
        if not isinstance(message, Mapping):
            return _error_response(None, -32600, "Invalid Request")
        request_id = message.get("id")
        if message.get("jsonrpc") != "2.0" or not isinstance(message.get("method"), str):
            return _error_response(request_id, -32600, "Invalid Request")
        method = str(message["method"])
        if "id" not in message:
            return None
        params = message.get("params", {})
        if not isinstance(params, Mapping):
            return _error_response(request_id, -32602, "params must be an object")
        try:
            result = self._dispatch(method, params)
            return {"jsonrpc": "2.0", "id": request_id, "result": result}
        except RpcError as exc:
            return _error_response(request_id, exc.code, exc.message, exc.data)
        except Exception as exc:
            return _error_response(request_id, -32603, "Internal error", str(exc))


def _error_response(
    request_id: Any, code: int, message: str, data: Any = None
) -> dict[str, Any]:
    error: dict[str, Any] = {"code": code, "message": message}
    if data is not None:
        error["data"] = data
    return {"jsonrpc": "2.0", "id": request_id, "error": error}


def _handle_wire_value(server: FullbleedMcpServer, value: Any) -> Any:
    if isinstance(value, list):
        if not value:
            return _error_response(None, -32600, "Invalid Request")
        responses = [server.handle_message(item) for item in value]
        filtered = [response for response in responses if response is not None]
        return filtered or None
    return server.handle_message(value)


def serve_stdio(root: Path | str) -> None:
    """Serve newline-delimited UTF-8 JSON-RPC messages on stdin/stdout."""
    server = FullbleedMcpServer(root)
    for raw_line in sys.stdin:
        line = raw_line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as exc:
            response = _error_response(None, -32700, "Parse error", str(exc))
        else:
            response = _handle_wire_value(server, message)
        if response is not None:
            sys.stdout.write(json.dumps(response, ensure_ascii=True) + "\n")
            sys.stdout.flush()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="fullbleed-mcp")
    parser.add_argument(
        "--root",
        default=".",
        help="Workspace root that confines document file access (default: current directory)",
    )
    arguments = parser.parse_args(argv)
    serve_stdio(Path(arguments.root))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())


__all__ = ["FullbleedMcpServer", "main", "serve_stdio"]
