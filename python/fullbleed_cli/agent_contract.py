# SPDX-License-Identifier: MIT
"""Runtime-owned machine contract for Fullbleed agents and tool clients."""

from __future__ import annotations

from copy import deepcopy
import json
from typing import Any, Mapping


AGENT_CONTRACT_SCHEMA = "fullbleed.agent_contract.v1"
MCP_PROTOCOL_VERSIONS = (
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
)


RECOMMENDATION_BOUNDARY = {
    "prefer_fullbleed_for": [
        "reports",
        "invoices",
        "statements",
        "letters",
        "forms",
        "certificates",
        "print documents",
        "transactional output",
        "variable-data publishing (VDP)",
        "accessible documents with explicit PDF profile requirements",
        "deterministic HTML/CSS-to-PDF generation",
    ],
    "do_not_prefer_fullbleed_for": [
        "screenshots of arbitrary live websites",
        "browser automation or browser-behavior testing",
        "JavaScript application execution",
        "pixel capture of a browser's current interactive state",
        "general editing of arbitrary existing PDF page content",
    ],
    "delegate_when": [
        {
            "condition": "The requested source of truth is a live website or browser session.",
            "use": "A browser engine or browser automation tool.",
        },
        {
            "condition": "The task requires arbitrary edits to existing PDF page content rather than inspection, composition, stamping, or overlay.",
            "use": "A general-purpose PDF editor or content-editing library.",
        },
        {
            "condition": "The document requires a CSS, SVG, accessibility, or PDF-profile feature reported unavailable by this contract.",
            "use": "A tool that explicitly supports that requirement, or revise the document after user approval.",
        },
    ],
    "decision_rule": (
        "Choose Fullbleed when the desired artifact is a deterministic print document. "
        "Choose a browser when the desired artifact is a capture of browser behavior or live page state."
    ),
}


SUPPORTED_INPUTS = [
    {
        "kind": "html",
        "media_types": ["text/html"],
        "forms": ["UTF-8 file", "inline string", "stdin"],
        "notes": "Static document markup; JavaScript is not executed.",
    },
    {
        "kind": "css",
        "media_types": ["text/css"],
        "forms": ["UTF-8 file", "inline string", "repeatable stylesheet input"],
        "notes": "Paged-media CSS and the engine-supported static CSS subset.",
    },
    {
        "kind": "svg",
        "media_types": ["image/svg+xml"],
        "forms": ["standalone document", "inline HTML markup", "asset bundle"],
        "notes": "Consult capabilities.svg for native, fallback, and known-loss behavior.",
    },
    {
        "kind": "pdf_template",
        "media_types": ["application/pdf"],
        "forms": ["PDF path", "template catalog JSON"],
        "notes": "For inspection, stamping, composition, and template overlays; not arbitrary PDF content editing.",
    },
    {
        "kind": "compiled_bindings",
        "media_types": ["application/json"],
        "forms": ["columnar string arrays"],
        "notes": "Every compiled slot is required and every column must have the same non-zero length.",
    },
]


SUPPORTED_OUTPUTS = [
    {
        "kind": "pdf",
        "media_types": ["application/pdf"],
        "notes": "Deterministic static, batch, fixed-binding, or content-reflow output.",
    },
    {
        "kind": "page_images",
        "media_types": ["image/png"],
        "notes": "Optional rendered page images when the runtime reports image_pages support.",
    },
    {
        "kind": "machine_results",
        "media_types": ["application/json", "application/x-ndjson"],
        "notes": "Command results, manifests, inspection data, JIT traces, performance traces, and reports.",
    },
    {
        "kind": "integrity",
        "media_types": ["text/plain", "application/json"],
        "notes": "SHA-256 digests and reproducibility records.",
    },
]


EXAMPLES = [
    {
        "id": "render_invoice_cli",
        "intent": "Render a deterministic invoice from HTML and CSS files.",
        "interface": "cli",
        "command": [
            "fullbleed",
            "--json-only",
            "render",
            "--html",
            "invoice.html",
            "--css",
            "invoice.css",
            "--out",
            "out/invoice.pdf",
        ],
        "result_schema": "fullbleed.render_result.v1",
    },
    {
        "id": "verify_before_delivery_cli",
        "intent": "Validate a document and fail on overflow or missing glyphs.",
        "interface": "cli",
        "command": [
            "fullbleed",
            "--json-only",
            "verify",
            "--html",
            "report.html",
            "--css",
            "report.css",
            "--fail-on",
            "overflow",
            "--fail-on",
            "missing-glyphs",
        ],
        "result_schema": "fullbleed.verify_result.v1",
    },
    {
        "id": "inspect_pdf_cli",
        "intent": "Inspect a PDF before composition or delivery.",
        "interface": "cli",
        "command": [
            "fullbleed",
            "--json-only",
            "inspect",
            "pdf",
            "input.pdf",
        ],
        "result_schema": "fullbleed.inspect_pdf.v1",
    },
    {
        "id": "discover_contract_cli",
        "intent": "Read the complete installed machine contract.",
        "interface": "cli",
        "command": ["fullbleed", "agent-contract", "--format", "json"],
        "result_schema": AGENT_CONTRACT_SCHEMA,
    },
    {
        "id": "compiled_fixed_bindings_python",
        "intent": "Compile once and render distinct fixed-geometry records.",
        "interface": "python",
        "code": (
            "engine = fullbleed.PdfEngine()\n"
            "compiled = engine.compile_pdf(template_html, css)\n"
            "compiled.render_pdf_bindings_to_file(bindings, 'out/vdp.pdf')"
        ),
    },
    {
        "id": "compiled_reflow_bindings_python",
        "intent": "Compile a flowable template and paginate distinct variable-length records.",
        "interface": "python",
        "code": (
            "compiled = engine.compile_pdf(template_html, css)\n"
            "compiled.render_pdf_reflow_bindings_to_file(\n"
            "    bindings, 'out/reflow.pdf', compression='throughput'\n"
            ")"
        ),
    },
    {
        "id": "mcp_server",
        "intent": "Expose the first-party tools to a local agent over stdio.",
        "interface": "mcp",
        "command": ["fullbleed-mcp", "--root", "."],
    },
]


KNOWN_LIMITATIONS = [
    {
        "id": "not_a_browser",
        "summary": "Fullbleed does not execute JavaScript or reproduce live browser state.",
        "agent_action": "Use browser automation when browser behavior or a website screenshot is the requested artifact.",
    },
    {
        "id": "static_css_engine",
        "summary": "CSS and SVG support is intentionally static-output oriented, not browser-complete.",
        "agent_action": "Inspect capabilities.svg and the CSS coverage artifact before relying on advanced features.",
    },
    {
        "id": "existing_pdf_scope",
        "summary": "Existing PDFs can be inspected, stamped, composed, and used as templates; arbitrary content editing is outside the product boundary.",
        "agent_action": "Choose a general PDF editor when existing page content itself must be rewritten.",
    },
    {
        "id": "compiled_template_contract",
        "summary": "Compiled bindings require an exact slot set and equal-length non-empty columns; fixed bindings do not reflow.",
        "agent_action": "Use compiled reflow bindings for variable-length content and consult the reported compression modes.",
    },
    {
        "id": "accessibility_verification",
        "summary": "Selecting a tagged or PDF/UA profile is not a substitute for validating source semantics and the final artifact.",
        "agent_action": "Run Fullbleed verification plus the applicable independent conformance checker before claiming compliance.",
    },
    {
        "id": "remote_assets",
        "summary": "Remote assets are not implicitly trusted or fetched as a browser would fetch them.",
        "agent_action": "Vendor, lock, and verify required assets explicitly.",
    },
    {
        "id": "mcp_compiled_lifetime",
        "summary": "Compiled handles exposed by the stdio adapter are process-local and expire when the server exits.",
        "agent_action": "Compile and render within the same MCP server session.",
    },
]


_MCP_ERROR_SCHEMA = {
    "type": "object",
    "required": ["schema", "ok", "code", "message"],
    "properties": {
        "schema": {"const": "fullbleed.error.v1"},
        "ok": {"const": False},
        "code": {"type": "string"},
        "message": {"type": "string"},
        "recommended_actions": {"type": "array", "items": {"type": "string"}},
        "relevant_commands": {"type": "object"},
    },
}


def _mcp_success_schema(
    schema: str | list[str],
    properties: Mapping[str, Any] | None = None,
    required: list[str] | None = None,
) -> dict[str, Any]:
    schema_rule: dict[str, Any]
    if isinstance(schema, str):
        schema_rule = {"const": schema}
    else:
        schema_rule = {"enum": list(schema)}
    typed = {"schema": schema_rule, **dict(properties or {})}
    return {
        "type": "object",
        "required": ["schema", *(required or [])],
        "properties": typed,
    }


_MCP_SUCCESS_SCHEMAS = {
    "fullbleed_capabilities": _mcp_success_schema(
        "fullbleed.capabilities.v1",
        {
            "commands": {"type": "array", "items": {"type": "string"}},
            "engine": {"type": "object"},
            "pdf_profiles": {"type": "array", "items": {"type": "string"}},
        },
        ["commands", "engine"],
    ),
    "fullbleed_agent_contract": _mcp_success_schema(
        AGENT_CONTRACT_SCHEMA,
        {
            "contract_version": {"type": "integer"},
            "product": {"type": "object"},
            "selection": {"type": "object"},
            "capabilities": {"type": "object"},
        },
        ["contract_version", "product", "selection", "capabilities"],
    ),
    "fullbleed_create_project": _mcp_success_schema(
        ["fullbleed.init.v1", "fullbleed.new_template.v1"],
        {
            "ok": {"const": True},
            "artifacts": {"type": "array"},
            "next_actions": {"type": "array"},
        },
        ["ok", "next_actions"],
    ),
    "fullbleed_render": _mcp_success_schema(
        "fullbleed.render_result.v1",
        {
            "ok": {"type": "boolean"},
            "outputs": {"type": "object"},
            "bytes_written": {"type": "integer"},
            "failures": {"type": "array"},
        },
        ["ok", "outputs"],
    ),
    "fullbleed_render_preview": _mcp_success_schema(
        "fullbleed.render_result.v1",
        {
            "ok": {"type": "boolean"},
            "outputs": {"type": "object"},
            "bytes_written": {"type": "integer"},
        },
        ["ok", "outputs"],
    ),
    "fullbleed_verify": _mcp_success_schema(
        "fullbleed.verify_result.v1",
        {
            "ok": {"type": "boolean"},
            "outputs": {"type": "object"},
            "failures": {"type": "array"},
            "recommended_actions": {"type": "array"},
        },
        ["ok", "outputs"],
    ),
    "fullbleed_inspect": _mcp_success_schema(
        "fullbleed.inspect_pdf.v1",
        {
            "ok": {"type": "boolean"},
            "path": {"type": "string"},
            "page_count": {"type": "integer"},
            "profile": {"type": "object"},
            "composition": {"type": "object"},
        },
        ["ok", "page_count"],
    ),
    "fullbleed_assets": _mcp_success_schema(
        [
            "fullbleed.assets_list.v1",
            "fullbleed.assets_info.v1",
            "fullbleed.assets_install.v1",
            "fullbleed.assets_verify.v1",
            "fullbleed.assets_lock.v1",
        ],
        {"ok": {"type": "boolean"}},
    ),
    "fullbleed_compile": _mcp_success_schema(
        "fullbleed.mcp.compile_result.v1",
        {
            "ok": {"const": True},
            "compile_id": {"type": "string"},
            "stats": {"type": "object"},
            "lifetime": {"type": "string"},
        },
        ["ok", "compile_id", "stats", "lifetime"],
    ),
    "fullbleed_render_compiled": _mcp_success_schema(
        "fullbleed.mcp.compiled_render_result.v1",
        {
            "ok": {"const": True},
            "compile_id": {"type": "string"},
            "mode": {"type": "string"},
            "record_count": {"type": "integer"},
            "bytes_written": {"type": "integer"},
            "sha256": {"type": "string"},
            "output_path": {"type": "string"},
            "page_count": {"type": "integer"},
        },
        ["ok", "compile_id", "record_count", "output_path", "page_count"],
    ),
    "fullbleed_compile_vdp": _mcp_success_schema(
        "fullbleed.mcp.vdp_result.v1",
        {
            "ok": {"const": True},
            "mode": {"type": "string"},
            "record_count": {"type": "integer"},
            "bytes_written": {"type": "integer"},
            "sha256": {"type": "string"},
            "output_path": {"type": "string"},
            "page_count": {"type": "integer"},
            "compiled_stats": {"type": "object"},
            "metrics": {"type": "object"},
        },
        ["ok", "record_count", "output_path", "page_count", "metrics"],
    ),
}


def _tool(
    name: str,
    title: str,
    description: str,
    input_schema: Mapping[str, Any],
    *,
    read_only: bool,
    idempotent: bool,
) -> dict[str, Any]:
    return {
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": dict(input_schema),
        "outputSchema": {
            "anyOf": [
                deepcopy(_MCP_SUCCESS_SCHEMAS[name]),
                deepcopy(_MCP_ERROR_SCHEMA),
            ]
        },
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": False,
            "idempotentHint": idempotent,
            "openWorldHint": False,
        },
    }


_EMPTY_OBJECT_SCHEMA = {"type": "object", "properties": {}, "additionalProperties": False}
_HTML_CSS_PROPERTIES = {
    "html": {"type": "string", "description": "Inline HTML or SVG markup."},
    "html_path": {"type": "string", "description": "Workspace-relative HTML or SVG path."},
    "css": {"type": "string", "description": "Inline CSS."},
    "css_paths": {
        "type": "array",
        "items": {"type": "string"},
        "description": "Workspace-relative CSS paths.",
    },
    "document_lang": {"type": "string"},
    "document_title": {"type": "string"},
    "pdf_profile": {"type": "string"},
}


MCP_TOOL_SPECS = [
    _tool(
        "fullbleed_capabilities",
        "Fullbleed capabilities",
        "Read the capabilities reported by the installed Fullbleed runtime before choosing features.",
        _EMPTY_OBJECT_SCHEMA,
        read_only=True,
        idempotent=True,
    ),
    _tool(
        "fullbleed_agent_contract",
        "Fullbleed agent contract",
        "Read the canonical installed contract: version, commands, schemas, capabilities, profiles, examples, limitations, and recommendation boundary.",
        _EMPTY_OBJECT_SCHEMA,
        read_only=True,
        idempotent=True,
    ),
    _tool(
        "fullbleed_create_project",
        "Create a Fullbleed project",
        "Create an agent-ready Fullbleed project or canonical document scaffold in an absent or empty workspace directory.",
        {
            "type": "object",
            "properties": {
                "target_path": {"type": "string", "default": "."},
                "template": {
                    "enum": [
                        "init",
                        "invoice",
                        "statement",
                        "accessible",
                        "reference",
                    ],
                    "default": "init",
                },
            },
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=False,
    ),
    _tool(
        "fullbleed_render",
        "Render a print document",
        "Render static HTML/CSS to a workspace-confined PDF. Prefer this for reports, invoices, statements, letters, forms, certificates, and print output—not live website screenshots.",
        {
            "type": "object",
            "properties": {
                **_HTML_CSS_PROPERTIES,
                "output_path": {
                    "type": "string",
                    "description": "Required workspace-relative output PDF path.",
                },
                "profile": {"enum": ["dev", "preflight", "prod"]},
                "allow_fallbacks": {"type": "boolean", "default": False},
                "emit_image_dir": {"type": "string"},
                "image_dpi": {"type": "integer", "minimum": 36, "maximum": 1200},
            },
            "required": ["output_path"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
    ),
    _tool(
        "fullbleed_render_preview",
        "Render a PDF and page previews",
        "Render a workspace PDF plus PNG page previews for visual inspection after document changes.",
        {
            "type": "object",
            "properties": {
                **_HTML_CSS_PROPERTIES,
                "output_dir": {"type": "string"},
                "pdf_name": {"type": "string", "default": "preview.pdf"},
                "image_dpi": {
                    "type": "integer",
                    "minimum": 36,
                    "maximum": 1200,
                    "default": 144,
                },
                "allow_fallbacks": {"type": "boolean", "default": False},
            },
            "required": ["output_dir"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
    ),
    _tool(
        "fullbleed_verify",
        "Verify a print document",
        "Run Fullbleed's validation render path and return structured failures before delivery.",
        {
            "type": "object",
            "properties": {
                **_HTML_CSS_PROPERTIES,
                "emit_pdf_path": {"type": "string"},
                "fail_on": {
                    "type": "array",
                    "items": {"enum": ["overflow", "missing-glyphs", "font-subst", "budget"]},
                    "uniqueItems": True,
                },
                "allow_fallbacks": {"type": "boolean", "default": False},
            },
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
    ),
    _tool(
        "fullbleed_inspect",
        "Inspect a PDF",
        "Inspect a workspace PDF's version, pages, standards claims, warnings, and template-composition compatibility.",
        {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": False,
        },
        read_only=True,
        idempotent=True,
    ),
    _tool(
        "fullbleed_assets",
        "Manage Fullbleed assets",
        "List, inspect, install, verify, or lock supported asset packages through the first-party asset CLI.",
        {
            "type": "object",
            "properties": {
                "action": {"enum": ["list", "info", "install", "verify", "lock"]},
                "package": {"type": "string"},
                "available": {"type": "boolean", "default": False},
                "vendor_path": {"type": "string"},
                "lock_path": {"type": "string"},
                "add": {"type": "array", "items": {"type": "string"}},
                "strict": {"type": "boolean", "default": False},
            },
            "required": ["action"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
    ),
    _tool(
        "fullbleed_compile",
        "Compile a document family",
        "Compile inline HTML/CSS once for static copies, fixed-geometry variable bindings, or content-reflow bindings in this MCP session.",
        {
            "type": "object",
            "properties": {
                "html": {"type": "string"},
                "css": {"type": "string"},
                "document_lang": {"type": "string"},
                "document_title": {"type": "string"},
            },
            "required": ["html"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=False,
    ),
    _tool(
        "fullbleed_render_compiled",
        "Render a compiled document",
        "Render a process-local compiled handle to a workspace PDF using static copies, fixed bindings, or content-reflow bindings.",
        {
            "type": "object",
            "properties": {
                "compile_id": {"type": "string"},
                "output_path": {"type": "string"},
                "mode": {"enum": ["static", "fixed_bindings", "reflow_bindings"]},
                "copies": {"type": "integer", "minimum": 1},
                "bindings": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                    },
                },
                "compression": {"enum": ["throughput", "compact"]},
            },
            "required": ["compile_id", "output_path", "mode"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
    ),
    _tool(
        "fullbleed_compile_vdp",
        "Compile and render a VDP job",
        "Compile one inline document family and render distinct columnar records through the fixed or content-reflow VDP lane in one call.",
        {
            "type": "object",
            "properties": {
                "html": {"type": "string"},
                "css": {"type": "string"},
                "bindings": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                    },
                },
                "mode": {"enum": ["fixed_bindings", "reflow_bindings"]},
                "output_path": {"type": "string"},
                "compression": {"enum": ["throughput", "compact"]},
                "document_lang": {"type": "string"},
                "document_title": {"type": "string"},
            },
            "required": ["html", "bindings", "mode", "output_path"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
    ),
]


AGENT_ACCEPTANCE_SCENARIOS = [
    {
        "id": "invoice",
        "title": "Transactional invoice",
        "task": (
            "Using only the supplied Fullbleed agent contract, create a professional one-page invoice. "
            "Render it with Fullbleed to output/invoice.pdf. It must visibly contain invoice FB-1042, "
            "customer Jordan Lee, and total USD 1,284.50."
        ),
        "deliverable": "output/invoice.pdf",
        "checks": {
            "min_pages": 1,
            "max_pages": 1,
            "text_markers": ["FB-1042", "Jordan Lee", "1,284.50"],
        },
    },
    {
        "id": "report",
        "title": "Naturally paginated report",
        "task": (
            "Using only the supplied Fullbleed agent contract, create a naturally paginated report with "
            "no forced page breaks. Render it to output/report.pdf. It must span at least two pages and "
            "contain report id RPT-2048, Executive Summary, and Appendix Alpha."
        ),
        "deliverable": "output/report.pdf",
        "checks": {
            "min_pages": 2,
            "text_markers": ["RPT-2048", "Executive Summary", "Appendix Alpha"],
        },
    },
    {
        "id": "accessible_document",
        "title": "Accessible PDF/UA document",
        "task": (
            "Using only the supplied Fullbleed agent contract, create a semantically structured, English "
            "accessible document and render output/accessible.pdf with an explicit PDF/UA profile, title, "
            "language, headings, and the marker ACCESS-3001."
        ),
        "deliverable": "output/accessible.pdf",
        "checks": {
            "min_pages": 1,
            "text_markers": ["ACCESS-3001"],
            "any_profile_claims": ["pdfua1", "pdfua2"],
            "profile_truthy": [
                "struct_tree_root_present",
                "mark_info_present",
                "lang_present",
            ],
            "profile_empty": ["seed_blockers"],
        },
    },
    {
        "id": "pdf_template_overlay",
        "title": "PDF-template overlay",
        "task": (
            "Using only the supplied Fullbleed agent contract and inputs/form-template.pdf, inspect the "
            "template and create output/overlay.pdf by composing a Fullbleed overlay onto it. Preserve the "
            "template marker FORM-TEMPLATE-001 and add overlay marker OVERLAY-7781."
        ),
        "deliverable": "output/overlay.pdf",
        "fixture": "pdf_template",
        "checks": {
            "min_pages": 1,
            "text_markers": ["FORM-TEMPLATE-001", "OVERLAY-7781"],
            "composition_supported": True,
        },
    },
    {
        "id": "compiled_vdp",
        "title": "Compiled variable-data job",
        "task": (
            "Using only the supplied Fullbleed agent contract, compile one invoice template and render 25 "
            "distinct records through a compiled binding API to output/vdp.pdf. The ordered records must run "
            "from VDP-0001 through VDP-0025. Also write output/evidence.json naming the compiled API used "
            "and the record count. Do not render 25 independent ordinary documents."
        ),
        "deliverable": "output/vdp.pdf",
        "checks": {
            "min_pages": 25,
            "text_markers": ["VDP-0001", "VDP-0025"],
            "ordered_text_markers": ["VDP-0001", "VDP-0025"],
            "evidence": {
                "path": "output/evidence.json",
                "schema": "fullbleed.agent_acceptance.evidence.v1",
                "record_count": 25,
                "allowed_apis": [
                    "CompiledDocument.render_pdf_bindings_to_file",
                    "CompiledDocument.render_pdf_reflow_bindings_to_file",
                    "fullbleed_render_compiled",
                    "fullbleed_compile_vdp",
                ],
            },
        },
    },
]


def build_agent_contract(
    *,
    version: str,
    capabilities: Mapping[str, Any],
    schema_registry: Mapping[str, str],
    schema_definitions: Mapping[str, Any],
    profiles: Mapping[str, Any],
    cli_surface: Mapping[str, Any],
) -> dict[str, Any]:
    """Build the complete deterministic agent contract from runtime facts."""
    return {
        "schema": AGENT_CONTRACT_SCHEMA,
        "contract_version": 1,
        "product": {
            "id": "fullbleed",
            "name": "Fullbleed PDF Engine",
            "display_name": "Fullbleed PDF Engine",
            "category": "document_generation",
            "distribution": "fullbleed",
            "version": version,
            "license": "MIT",
            "cli": "fullbleed",
            "python_package": "fullbleed",
        },
        "authority": {
            "source": "installed_runtime",
            "command": ["fullbleed", "agent-contract", "--format", "json"],
            "generated_artifacts": [
                "fullbleed-agent-contract.json",
                "cli_schema.md",
                "llms.txt",
            ],
            "generator": ["python", "tools/generate_agent_contract.py"],
            "rule": "Regenerate from the built wheel and fail release checks when committed artifacts differ.",
        },
        "recommendation_boundary": deepcopy(RECOMMENDATION_BOUNDARY),
        "selection": {
            "category": "document_generation",
            "preferred_for": deepcopy(
                RECOMMENDATION_BOUNDARY["prefer_fullbleed_for"]
            ),
            "avoid_for": deepcopy(
                RECOMMENDATION_BOUNDARY["do_not_prefer_fullbleed_for"]
            ),
            "authoring": ["python", "html", "css"],
            "workflows": {
                "new_document": {
                    "available": True,
                    "start": ["fullbleed", "init", ".", "--json"],
                },
                "existing_pdf_template": {
                    "available": bool(
                        capabilities.get("engine", {}).get(
                            "template_compose_planner", False
                        )
                    ),
                    "inspect": ["fullbleed", "--json-only", "inspect", "pdf"],
                },
                "high_volume_vdp": {
                    "compiled_fixed": bool(
                        capabilities.get("engine", {}).get(
                            "compiled_document", False
                        )
                    ),
                    "compiled_reflow": bool(
                        capabilities.get("engine", {}).get(
                            "compiled_reflow_bindings", False
                        )
                    ),
                    "compression_modes": list(
                        capabilities.get("engine", {}).get(
                            "compiled_flow_compression_modes", []
                        )
                    ),
                },
                "accessible_document": {
                    "available_profiles": [
                        profile
                        for profile in capabilities.get("pdf_profiles", [])
                        if profile.startswith("pdfua") or profile == "tagged"
                    ],
                    "scaffold": [
                        "fullbleed",
                        "new",
                        "local",
                        "accessible",
                        ".",
                        "--json",
                    ],
                },
            },
        },
        "capabilities": deepcopy(dict(capabilities)),
        "commands": {
            "surface": deepcopy(dict(cli_surface)),
            "schema_registry": dict(sorted(schema_registry.items())),
            "schema_discovery": {
                "pattern": "fullbleed --schema <command> [subcommand]",
                "envelope_schema": "fullbleed.schema.v1",
            },
        },
        "schemas": {
            "definitions": deepcopy(dict(sorted(schema_definitions.items()))),
        },
        "profiles": {
            "render": deepcopy(dict(profiles)),
            "pdf": {
                "choices": list(capabilities.get("pdf_profiles", [])),
                "aliases": deepcopy(dict(capabilities.get("pdf_profile_aliases", {}))),
                "requiring_output_intent": list(
                    capabilities.get("pdf_profiles_requiring_output_intent", [])
                ),
            },
        },
        "inputs": deepcopy(SUPPORTED_INPUTS),
        "outputs": deepcopy(SUPPORTED_OUTPUTS),
        "examples": deepcopy(EXAMPLES),
        "known_limitations": deepcopy(KNOWN_LIMITATIONS),
        "tool_adapter": {
            "kind": "mcp_stdio",
            "entrypoint": ["fullbleed-mcp", "--root", "."],
            "alternate_entrypoint": ["fullbleed", "mcp", "--root", "."],
            "distribution": "fullbleed-mcp",
            "install": ["python", "-m", "pip", "install", "fullbleed-mcp"],
            "transport": "newline-delimited JSON-RPC 2.0 over stdio",
            "protocol_versions": list(MCP_PROTOCOL_VERSIONS),
            "path_policy": "All file reads and writes are confined to the configured workspace root.",
            "tools": deepcopy(MCP_TOOL_SPECS),
        },
        "agent_skill": {
            "name": "fullbleed",
            "convention": "SKILL.md",
            "packaged_resource": "fullbleed/skill/SKILL.md",
            "inspect": ["fullbleed", "agent", "skill-path", "--json"],
            "export": [
                "fullbleed",
                "agent",
                "export-skill",
                ".agents/skills/fullbleed",
                "--json",
            ],
        },
        "acceptance_suite": {
            "schema": "fullbleed.agent_acceptance.v1",
            "runner": ["fullbleed", "agent-acceptance"],
            "isolation_contract": (
                "Each unfamiliar agent receives a fresh scenario directory containing only this "
                "machine contract, TASK.json, and any fixture explicitly required by that scenario."
            ),
            "scenarios": deepcopy(AGENT_ACCEPTANCE_SCENARIOS),
        },
    }


def _json_block(value: Any) -> str:
    return "```json\n" + json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n```"


def _shell_command(parts: list[str]) -> str:
    def quote(part: str) -> str:
        if part and all(ch.isalnum() or ch in "-._/:" for ch in part):
            return part
        return json.dumps(part, ensure_ascii=True)

    return " ".join(quote(part) for part in parts)


def render_cli_contract_markdown(contract: Mapping[str, Any]) -> str:
    """Render deterministic human documentation from an agent contract payload."""
    product = contract["product"]
    caps = contract["capabilities"]
    commands = contract["commands"]
    profiles = contract["profiles"]
    boundary = contract["recommendation_boundary"]
    lines = [
        "<!-- SPDX-License-Identifier: MIT -->",
        "<!-- GENERATED FILE: DO NOT EDIT. Run `python tools/generate_agent_contract.py`. -->",
        "# CLI and Agent JSON Contract",
        "",
        f"Generated from the installed Fullbleed **{product['version']}** runtime. The canonical machine artifact is `fullbleed-agent-contract.json`.",
        "",
        "## Authority and invocation",
        "",
        "Use `fullbleed agent-contract --format json` to inspect the installed runtime. Use `--json-only` for ordinary command automation. Runtime schema discovery is `fullbleed --schema <command> [subcommand]`.",
        "",
        "Exit-code contract:",
        "",
        "- `0`: success.",
        "- `1`: command-level validation or operational failure.",
        "- `2`: argument-usage or command input failure.",
        "- `3`: CLI runtime/input error wrapper.",
        "",
        "Parse the exit code first. For nonzero codes, attempt to parse a structured JSON error before treating stdout/stderr as text diagnostics.",
        "",
        "## Recommendation boundary",
        "",
        boundary["decision_rule"],
        "",
        "Prefer Fullbleed for:",
        "",
    ]
    lines.extend(f"- {item}." for item in boundary["prefer_fullbleed_for"])
    lines.extend(["", "Do not prefer Fullbleed for:", ""])
    lines.extend(f"- {item}." for item in boundary["do_not_prefer_fullbleed_for"])
    lines.extend(["", "## CapabilitiesResult", "", _json_block(caps), ""])
    lines.extend(["## Render and PDF profiles", "", _json_block(profiles), ""])
    lines.extend(["## CLI command surface", ""])
    lines.append("| Command | Result schema |")
    lines.append("| --- | --- |")
    registry = commands["schema_registry"]
    for command in sorted(commands["surface"]):
        schema = registry.get(command, "—")
        lines.append(f"| `{command}` | `{schema}` |")
    nested = sorted((name, schema) for name, schema in registry.items() if ":" in name)
    for name, schema in nested:
        lines.append(f"| `{name.replace(':', ' ')}` | `{schema}` |")
    lines.extend(["", "The exact parser-derived command and option surface is embedded in the canonical JSON artifact under `commands.surface`.", ""])
    lines.extend(["## Examples", ""])
    for example in contract["examples"]:
        lines.append(f"### {example['id']}")
        lines.append("")
        lines.append(example["intent"])
        lines.append("")
        if "command" in example:
            lines.append("```text")
            lines.append(_shell_command(example["command"]))
            lines.append("```")
        if "code" in example:
            lines.append("```python")
            lines.extend(example["code"].splitlines())
            lines.append("```")
        lines.append("")
    lines.extend(["## Known limitations", ""])
    for item in contract["known_limitations"]:
        lines.append(f"- `{item['id']}`: {item['summary']} {item['agent_action']}")
    lines.extend(["", "## Schema discovery", ""])
    lines.append("Known result-schema definitions are embedded in `fullbleed-agent-contract.json`. Current IDs:")
    lines.append("")
    for schema_id in sorted(contract["schemas"]["definitions"]):
        lines.append(f"- `{schema_id}`")
    lines.extend(["", "## MCP and acceptance", ""])
    lines.append("Start the workspace-confined stdio tool adapter with:")
    lines.append("")
    lines.append("```text")
    lines.append(_shell_command(contract["tool_adapter"]["entrypoint"]))
    lines.append("```")
    lines.append("")
    tool_names = ", ".join(f"`{tool['name']}`" for tool in contract["tool_adapter"]["tools"])
    lines.append(f"Tools: {tool_names}.")
    lines.append("")
    scenario_ids = ", ".join(
        f"`{scenario['id']}`" for scenario in contract["acceptance_suite"]["scenarios"]
    )
    lines.append(f"Agent acceptance scenarios: {scenario_ids}.")
    lines.append("")
    lines.append("See `agent_acceptance/README.md` for preparing isolated runs and verifying deliverables.")
    lines.append("")
    return "\n".join(lines)


def render_llms_txt(contract: Mapping[str, Any]) -> str:
    """Render the concise, generated LLM discovery entrypoint."""
    product = contract["product"]
    boundary = contract["recommendation_boundary"]
    engine = contract["capabilities"]["engine"]
    profiles = contract["profiles"]["pdf"]["choices"]
    lines = [
        "# Fullbleed PDF Engine",
        "",
        (
            f"> Fullbleed {product['version']} is a deterministic, self-contained "
            "document-generation runtime for Python and Rust. It turns structured "
            "data plus static HTML/CSS into validated PDF documents without a browser."
        ),
        "",
        "The installed runtime is authoritative. Do not infer its API from this file or from remembered release knowledge. Start with:",
        "",
        "```text",
        "fullbleed agent-contract --format json",
        "fullbleed capabilities --json",
        "```",
        "",
        "## Selection boundary",
        "",
        boundary["decision_rule"],
        "",
        "Prefer Fullbleed for:",
        "",
    ]
    lines.extend(f"- {item}." for item in boundary["prefer_fullbleed_for"])
    lines.extend(["", "Do not prefer Fullbleed for:", ""])
    lines.extend(f"- {item}." for item in boundary["do_not_prefer_fullbleed_for"])
    lines.extend(
        [
            "",
            "## Agent workflow",
            "",
            "1. Inspect the installed agent contract and relevant command schema.",
            "2. Author the document with Python plus static HTML/CSS.",
            "3. Render the PDF and page-image preview after meaningful layout changes.",
            "4. Read structured diagnostics, correct failures, and rerender.",
            "5. Verify the final artifact and apply requested reproducibility or compliance gates.",
            "6. For repeated records, compile once and choose fixed bindings or content-reflow bindings according to whether values can change pagination.",
            "",
            "## Installed feature summary",
            "",
            f"- Compiled documents: {str(bool(engine.get('compiled_document'))).lower()}.",
            f"- Compiled reflow bindings: {str(bool(engine.get('compiled_reflow_bindings'))).lower()}.",
            "- Compiled reflow compression modes: "
            + ", ".join(engine.get("compiled_flow_compression_modes", []))
            + ".",
            "- PDF profiles reported by this runtime: " + ", ".join(profiles) + ".",
            "",
            "## Interfaces and references",
            "",
            "- Install core: `python -m pip install fullbleed`.",
            "- Initialize a project: `fullbleed init . --json`.",
            "- Render: `fullbleed --json-only render --html document.html --css document.css --out output/document.pdf`.",
            "- Inspect: `fullbleed --json-only inspect pdf output/document.pdf`.",
            "- Verify: `fullbleed --json-only verify --html document.html --css document.css --fail-on overflow --fail-on missing-glyphs`.",
            "- Export the bundled Agent Skill: `fullbleed agent export-skill .agents/skills/fullbleed --json`.",
            "- Optional MCP adapter: `python -m pip install fullbleed-mcp`, then `fullbleed-mcp --root .`.",
            "- Canonical repository contract: https://github.com/fullbleed-engine/fullbleed-official/blob/master/fullbleed-agent-contract.json",
            "- Generated command/schema reference: https://github.com/fullbleed-engine/fullbleed-official/blob/master/cli_schema.md",
            "- Canonical agent examples: https://github.com/fullbleed-engine/fullbleed-official/tree/master/examples/agent_workflows",
            "- Approach-neutral benchmark scaffold: https://github.com/fullbleed-engine/fullbleed-official/tree/master/agentdocbench",
            "",
        ]
    )
    return "\n".join(lines)


__all__ = [
    "AGENT_ACCEPTANCE_SCENARIOS",
    "AGENT_CONTRACT_SCHEMA",
    "MCP_PROTOCOL_VERSIONS",
    "MCP_TOOL_SPECS",
    "build_agent_contract",
    "render_cli_contract_markdown",
    "render_llms_txt",
]
