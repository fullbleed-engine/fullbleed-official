# SPDX-License-Identifier: MIT
"""Check installed component diagnostics and real Pyright LSP completions.

Test-only prerequisites: Node and the official pyright npm package. No editor,
browser, or language server is included in the Fullbleed runtime dependency set.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import queue
import runpy
import shutil
import subprocess
import sys
import tempfile
import threading
import time

import fullbleed.ui.experimental as components
from fullbleed.ui.experimental._schema import STYLE_PROPERTIES


class LanguageServer:
    def __init__(self, command, folder, environment):
        self.process = subprocess.Popen(
            command, cwd=folder, env=environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        self.messages = queue.Queue()
        self.errors = []
        self.sequence = 0
        self.settings = {"python": {"pythonPath": sys.executable, "analysis": {
            "typeCheckingMode": "strict", "autoSearchPaths": False,
            "diagnosticMode": "openFilesOnly", "useLibraryCodeForTypes": False,
        }}, "pyright": {}}
        threading.Thread(target=self.read_messages, daemon=True).start()
        threading.Thread(target=self.read_errors, daemon=True).start()
        try:
            self.request("initialize", {
                "processId": os.getpid(), "rootUri": folder.as_uri(),
                "capabilities": {"workspace": {"configuration": True},
                                 "textDocument": {"completion": {"completionItem": {"snippetSupport": True}}}},
                "workspaceFolders": [{"uri": folder.as_uri(), "name": "Installed consumer"}],
            })
            self.send({"method": "initialized", "params": {}})
            self.send({"method": "workspace/didChangeConfiguration", "params": {"settings": self.settings}})
        except BaseException:
            self.process.kill()
            self.process.wait(timeout=5)
            raise

    def read_errors(self):
        for line in self.process.stderr:
            self.errors.append(line.decode("utf-8", errors="replace").rstrip())

    def read_messages(self):
        try:
            while True:
                headers = {}
                while True:
                    line = self.process.stdout.readline()
                    if not line:
                        raise EOFError("Language server exited")
                    if line in (b"\r\n", b"\n"):
                        break
                    key, value = line.decode("ascii").split(":", 1)
                    headers[key.lower()] = value.strip()
                self.messages.put(json.loads(self.process.stdout.read(int(headers["content-length"]))))
        except Exception as error:
            self.messages.put(error)

    def send(self, message):
        payload = json.dumps({"jsonrpc": "2.0", **message}).encode("utf-8")
        self.process.stdin.write(f"Content-Length: {len(payload)}\r\n\r\n".encode("ascii") + payload)
        self.process.stdin.flush()

    def request(self, method, params):
        self.sequence += 1
        request_id = self.sequence
        self.send({"id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while True:
            try:
                message = self.messages.get(timeout=max(.01, deadline - time.monotonic()))
            except queue.Empty as error:
                raise TimeoutError(f"Language server request timed out: {method}; {self.errors[-10:]}") from error
            if isinstance(message, Exception):
                raise RuntimeError(self.errors[-20:]) from message
            if "method" in message and "id" in message:
                result = None
                if message["method"] == "workspace/configuration":
                    result = []
                    for item in message["params"]["items"]:
                        value = self.settings
                        for key in item.get("section", "").split("."):
                            value = value.get(key, {})
                        result.append(value)
                self.send({"id": message["id"], "result": result})
            elif message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(message["error"])
                return message.get("result")

    def open(self, path, text):
        self.send({"method": "textDocument/didOpen", "params": {"textDocument": {
            "uri": path.as_uri(), "languageId": "python", "version": 1, "text": text,
        }}})

    def close(self):
        try:
            if self.process.poll() is None:
                self.request("shutdown", None)
                self.send({"method": "exit", "params": None})
                self.process.wait(timeout=5)
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pyright-root", required=True, type=Path, help="Installed node_modules/pyright directory")
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    package = args.pyright_root.resolve()
    metadata = json.loads((package / "package.json").read_text(encoding="utf-8"))
    node = shutil.which("node")
    assert node, "Install Node as a test-only tool"
    root = Path(__file__).resolve().parents[1]
    assert root / "python" not in Path(components.__file__).parents, "Use an installed wheel with python -I"
    contract = runpy.run_path(str(root / "tools/smoke_component_typing.py"))
    invalid, expected_errors = contract["invalid_consumer"]()
    environment = {key: value for key, value in os.environ.items() if key not in {"PYTHONPATH", "MYPYPATH"}}
    completion_evidence = []
    with tempfile.TemporaryDirectory(prefix="fullbleed-editor-") as directory:
        folder = Path(directory).resolve()
        (folder / "valid.py").write_text(contract["VALID"], encoding="utf-8")
        (folder / "invalid.py").write_text(invalid, encoding="utf-8")
        (folder / "pyrightconfig.json").write_text(json.dumps({
            "typeCheckingMode": "strict", "pythonVersion": "3.10",
        }), encoding="utf-8")
        diagnostics = {}
        for name, exit_code in (("valid.py", 0), ("invalid.py", 1)):
            check = subprocess.run([node, str(package / metadata["bin"]["pyright"]),
                                    "--pythonpath", sys.executable, "--outputjson", name],
                                   cwd=folder, env=environment, capture_output=True, text=True, timeout=60)
            assert check.returncode == exit_code, (name, check.stdout, check.stderr)
            diagnostics[name] = json.loads(check.stdout)
        errors = diagnostics["invalid.py"]["generalDiagnostics"]
        lines = {item["range"]["start"]["line"] + 1 for item in errors if item["severity"] == "error"}
        missed = {label for line, label in expected_errors.items() if line not in lines}
        assert not missed - contract["RUNTIME_ONLY_CASES"] and not lines - set(expected_errors), (missed, errors)

        server = LanguageServer([node, str(package / metadata["bin"]["pyright-langserver"]), "--stdio"], folder, environment)
        try:
            cases = [
                ("style_properties", "style(§)", {name.replace("-", "_") for name in STYLE_PROPERTIES}),
                ("primitive_props", 'Paragraph("Text", cla§)', {"class_name"}),
                ("root_style", 'Paragraph("Text").with_style(font_§)', {"font_size", "font_family", "font_weight"}),
                ("style_recipe", "styled(Paragraph, pad§)", {"padding", "padding_top"}),
                ("layout_values", 'Stack(Paragraph("Text"), align="§")', {"start", "center", "stretch"}),
                ("table_columns", "DataTable([], col§)", {"columns"}),
                ("heading_keep", 'Heading("Text", keep_§)', {"keep_with_next"}),
                ("styled_business_argument", 'def Choice(*, style: str) -> Node:\n    return Paragraph(style)\nStyledChoice = styled(Choice, color="red")\nStyledChoice(sty§)', {"style"}),
                ("styled_layout_values", 'Aligned = styled(Row, align_items="end")\nAligned(Paragraph("Text"), align="§")', {"start", "center", "stretch"}),
                ("styled_grid_columns", 'Columns = styled(Grid, grid_template_columns="2fr 1fr")\nColumns(Paragraph("Text"), col§)', {"columns"}),
                ("styled_heading_keep", 'Title = styled(Heading, break_after="auto")\nTitle("Text", keep_§)', {"keep_with_next"}),
                ("prepared_outputs", 'Document(Paragraph("Text")).prepare(Renderer()).§', {"render_pdf", "render_image_pages", "export_render_time_pagination_trace"}),
                ("prepared_preview_dpi", 'Document(Paragraph("Text")).prepare(Renderer()).render_image_pages(d§)', {"dpi"}),
                ("node_theme_requirements", 'Paragraph("Text").required_§', {"required_tokens"}),
                ("document_theme_requirements", 'Document(Paragraph("Text")).required_§', {"required_tokens"}),
                ("theme_derivation", 'Document(Paragraph("Text")).theme.with_§', {"with_values"}),
            ]
            for index, (name, expression, required) in enumerate(cases):
                prefix = "from fullbleed.ui.experimental import style, styled, Paragraph, Stack, Row, Grid, DataTable, Heading, Node, Document, Renderer\n"
                before, after = (prefix + expression).split("§")
                path = folder / f"completion_{index}.py"
                server.open(path, before + after)
                result = server.request("textDocument/completion", {
                    "textDocument": {"uri": path.as_uri()},
                    "position": {"line": before.count("\n"), "character": len(before.rsplit("\n", 1)[-1])},
                    "context": {"triggerKind": 1},
                })
                items = result if isinstance(result, list) else (result or {}).get("items", [])
                labels = {item["label"].rstrip("= ").strip("\"'") for item in items}
                assert required <= labels, (name, sorted(required - labels), sorted(labels))
                completion_evidence.append({"name": name, "required": sorted(required), "matched": sorted(required & labels)})
        finally:
            server.close()
    result = {"ok": True, "pyright_version": metadata["version"], "runtime": str(components.__file__),
              "valid_consumer": diagnostics["valid.py"], "invalid_consumer": diagnostics["invalid.py"],
              "rejected_cases": len(expected_errors) - len(missed), "runtime_only_cases": sorted(missed),
              "completions": completion_evidence,
              "scope": "Installed wheel checked by the official Pyright CLI and language server. Not a claim for every editor or all dynamic CSS/token/part constraints."}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "rejected_cases": result["rejected_cases"], "completion_checks": len(completion_evidence)}))


if __name__ == "__main__":
    main()
