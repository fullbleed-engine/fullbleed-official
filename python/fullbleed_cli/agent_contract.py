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
        "id": "watch_document_cli",
        "intent": "Rebuild a local document after HTML/CSS edits until Ctrl-C; emits a stream of render results and errors.",
        "interface": "cli",
        "command": [
            "fullbleed", "--json-only", "render", "--html", "invoice.html",
            "--css", "invoice.css", "--out", "invoice.pdf", "--watch",
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
        "id": "watch_scope",
        "summary": "Render watch mode polls explicit local input files and optional recursive watch paths. It does not run a Python data generator or discover every file referenced inside HTML/CSS or JSON.",
        "agent_action": "Use --watch-path for extra dependencies. Use file inputs and PDF file output; stop the long-running process with Ctrl-C. Use one-shot render for delivery gates.",
    },
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
        "id": "pdfvt_verification",
        "summary": "PDF/VT-1 output has Job/Record/Document parts, private Fullbleed DPM, and conservative file-scoped reuse hints. Internal inspection does not establish ISO conformance or DFE performance.",
        "agent_action": "Supply a valid ICC, embedded fonts, title, and explicit job timestamp. Retain a dedicated PDF/VT validator report before claiming conformance; agree DPM semantics with the print provider.",
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
    open_world: bool = False,
    destructive: bool = False,
) -> dict[str, Any]:
    return {
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": dict(input_schema),
        "outputSchema": {
            "type": "object",
            "anyOf": [
                deepcopy(_MCP_SUCCESS_SCHEMAS[name]),
                deepcopy(_MCP_ERROR_SCHEMA),
            ]
        },
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "idempotentHint": idempotent,
            "openWorldHint": open_world,
        },
    }


_EMPTY_OBJECT_SCHEMA = {"type": "object", "properties": {}, "additionalProperties": False}
_PART_METADATA = {
    "type": "object",
    "description": "Private Fullbleed DPM: strings, signed 64-bit integers, finite reals within the PDF reader's single-precision range, booleans, arrays, and nested dictionaries; no nulls. At most 16 nesting levels and 10000 values per node.",
}
_PART_PROPERTIES = {
    "id": {
        "type": "string",
        "minLength": 1,
        "description": "Nonblank part identifier, at most 4096 UTF-8 bytes. Record IDs must be unique in the job; document IDs must be unique within their record.",
    },
    "metadata": _PART_METADATA,
}
PDF_VT_JOB_SCHEMA = {
    "type": "object",
    "description": "Optional Job/Record/Document hierarchy and private Fullbleed DPM for pdf_profile='pdfvt1'. Omit records to generate one record and document per input, compiled copy, or binding row.",
    "properties": {
        **_PART_PROPERTIES,
        "records": {
            "type": "array",
            "description": "Ordered records; omitted or empty generates one record per input document, compiled copy, or binding row.",
            "items": {
                "type": "object",
                "properties": {
                    **_PART_PROPERTIES,
                    "documents": {
                        "type": "array", "minItems": 1,
                        "description": "Ordered documents in this record. Each consumes one input document, compiled copy, or binding row; final pagination supplies its page range.",
                        "items": {"type": "object", "properties": _PART_PROPERTIES,
                                  "required": ["id"], "additionalProperties": False},
                    },
                },
                "required": ["id", "documents"], "additionalProperties": False,
            },
        },
    },
    "required": ["id"], "additionalProperties": False,
}
_ENGINE_PROPERTIES = {
    "document_lang": {
        "type": "string",
        "description": "Document language for PDF metadata, as a BCP-47 tag such as 'en-US'.",
    },
    "document_title": {
        "type": "string",
        "description": "Human-readable title stored in PDF metadata. Required for pdfx4 and pdfvt1 output.",
    },
    "document_timestamp": {
        "type": "string",
        "description": "PDF write date: UTC YYYY-MM-DDTHH:MM:SSZ, 'current', or 'source-date-epoch' (reads SOURCE_DATE_EPOCH). Required for pdfx4 and pdfvt1. Resolved once per engine; use an explicit value for repeatable output.",
    },
    "pdf_profile": {
        "type": "string",
        "description": "PDF export profile; omitted means ordinary PDF. Read fullbleed_capabilities.pdf_profile_catalog for names and font, ICC, title, and timestamp requirements. This is separate from the render tool's dev/preflight/prod preset.",
    },
    "pdf_vt_job": PDF_VT_JOB_SCHEMA,
    "output_intent_icc_path": {
        "type": "string",
        "description": "Existing ICC profile file under the MCP workspace root, used as the PDF output intent. Required by PDF/A, PDF/X, and PDF/VT profiles.",
    },
    "output_intent_identifier": {
        "type": "string",
        "description": "Nonblank output-condition identifier for the ICC output intent; defaults to 'Custom'. Requires output_intent_icc_path.",
    },
    "output_intent_info": {
        "type": "string",
        "description": "Optional human-readable description of the output condition. Requires output_intent_icc_path.",
    },
    "output_intent_components": {
        "enum": [1, 3, 4],
        "description": "ICC color-component count: 1 for gray, 3 for RGB, 4 for CMYK. Defaults to 3 and must match the profile supplied by output_intent_icc_path.",
    },
    "font_paths": {
        "type": "array", "items": {"type": "string"},
        "description": "Existing embeddable font files under the MCP workspace root. Register these fonts before rendering; do not rely on system fonts. Relative paths are resolved from the server's --root.",
    },
}
_HTML_CSS_PROPERTIES = {
    **_ENGINE_PROPERTIES,
    "html": {"type": "string", "description": "Inline static HTML or SVG markup. Supply exactly one of html and html_path; JavaScript is not executed."},
    "html_path": {"type": "string", "description": "Existing UTF-8 HTML or SVG file under the MCP workspace root. Relative to the server's --root. Supply exactly one of html and html_path."},
    "css": {"type": "string", "description": "Optional inline CSS, applied before css_paths. Use @page for print page size and margins."},
    "css_paths": {
        "type": "array",
        "items": {"type": "string"},
        "description": "Existing UTF-8 CSS files under the MCP workspace root, read in array order after inline css. Relative paths are resolved from the server's --root.",
    },
}
_COMPILED_SOURCE_PROPERTIES = {
    "html": {
        "type": "string",
        "description": "Nonempty inline static HTML. For variable records use {{slot_name}} placeholders and supply a bindings column for every slot. Compilation tools accept markup, not html_path.",
    },
    "css": {
        "type": "string",
        "description": "Optional inline print CSS; defaults to an empty stylesheet. Use @page for page size and margins. Compilation tools do not accept css_paths.",
    },
    **_ENGINE_PROPERTIES,
}
_OUTPUT_PDF_PATH = {
    "type": "string",
    "description": "Destination PDF under the MCP workspace root, relative to the server's --root. Parent directories are created; an existing file is replaced.",
}
_ALLOW_FALLBACKS = {
    "type": "boolean", "default": False,
    "description": "Allow missing-glyph and font-substitution signals without failing those selected fail_on checks. Does not enable a browser fallback or fetch fonts.",
}
_BINDINGS = {
    "type": "object",
    "description": 'Columnar records, for example {"name": ["Ada", "Lin"]}. Keys must match the compiled {{slot_name}} set exactly. Every column must be a nonempty string array of equal length; values at the same index form one record. Required for fixed_bindings and reflow_bindings; ignored in static mode.',
    "additionalProperties": {
        "type": "array",
        "items": {"type": "string"},
        "minItems": 1,
    },
}
_COMPRESSION = {
    "enum": ["throughput", "compact"],
    "description": "Compression strategy for reflow_bindings only; defaults to throughput. Use compact for smaller output at additional compression cost. Ignored in static and fixed_bindings modes.",
}


MCP_TOOL_SPECS = [
    _tool(
        "fullbleed_capabilities",
        "Fullbleed capabilities",
        "Read the installed runtime's compact feature map and PDF profile requirements before choosing features. Use fullbleed_agent_contract for command schemas, workflow examples, and detailed limitations.",
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
        "Create an agent-ready project or document scaffold in an absent or empty workspace directory; returns generated artifacts and next actions. Use this when starting a project, then render its source with fullbleed_render or fullbleed_render_preview.",
        {
            "type": "object",
            "properties": {
                "target_path": {
                    "type": "string", "default": ".",
                    "description": "Destination directory under the MCP workspace root. Defaults to that root and must be absent or empty; use a new subdirectory in an existing project.",
                },
                "template": {
                    "enum": [
                        "init",
                        "invoice",
                        "statement",
                        "accessible",
                        "reference",
                    ],
                    "default": "init",
                    "description": "Scaffold to create: init supplies the general agent-ready project; invoice, statement, accessible, and reference select the corresponding document starter.",
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
        "Render static HTML/CSS to a PDF and return output paths and render diagnostics. Supply exactly one of html or html_path. Use fullbleed_render_preview for visual iteration, fullbleed_verify for delivery checks, or fullbleed_compile_vdp for variable records. Does not capture live websites.",
        {
            "type": "object",
            "properties": {
                **_HTML_CSS_PROPERTIES,
                "output_path": _OUTPUT_PDF_PATH,
                "profile": {
                    "enum": ["dev", "preflight", "prod"],
                    "description": "Optional render preset: dev enables JIT planning without XObject reuse; preflight enables planning and reuse; prod enables reuse with JIT off. Omit to use engine defaults. Select PDF standards separately with pdf_profile.",
                },
                "allow_fallbacks": _ALLOW_FALLBACKS,
                "emit_image_dir": {
                    "type": "string",
                    "description": "Optional directory under the MCP workspace root for per-page PNG previews. Omit for PDF-only output; fullbleed_render_preview chooses the PDF and PNG paths together.",
                },
                "image_dpi": {
                    "type": "integer", "minimum": 36, "maximum": 1200,
                    "description": "PNG preview resolution in dots per inch; defaults to 150. Used only when emit_image_dir is supplied; does not change PDF page geometry.",
                },
            },
            "required": ["output_path"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
        destructive=True,
    ),
    _tool(
        "fullbleed_render_preview",
        "Render a PDF and page previews",
        "Render a PDF plus PNG page previews for visual inspection after document changes; returns their artifact paths. Supply exactly one of html or html_path. Use fullbleed_render for PDF-only output and fullbleed_verify for explicit failure checks.",
        {
            "type": "object",
            "properties": {
                **_HTML_CSS_PROPERTIES,
                "output_dir": {
                    "type": "string",
                    "description": "Destination directory under the MCP workspace root. Writes pdf_name here and per-page PNGs in its pages subdirectory; directories are created as needed and matching files replaced.",
                },
                "pdf_name": {
                    "type": "string", "default": "preview.pdf",
                    "description": "Single PDF filename within output_dir, without directory components; defaults to preview.pdf.",
                },
                "image_dpi": {
                    "type": "integer",
                    "minimum": 36,
                    "maximum": 1200,
                    "default": 144,
                    "description": "PNG preview resolution in dots per inch; defaults to 144. Higher values increase image dimensions and rendering cost without changing PDF page geometry.",
                },
                "allow_fallbacks": _ALLOW_FALLBACKS,
            },
            "required": ["output_dir"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
        destructive=True,
    ),
    _tool(
        "fullbleed_verify",
        "Verify a print document",
        "Render HTML/CSS through Fullbleed's validation path and return structured diagnostics and failures before delivery. Supply exactly one of html or html_path and select fail_on checks explicitly. Use fullbleed_inspect for an existing PDF. Internal checks do not establish independent standards conformance.",
        {
            "type": "object",
            "properties": {
                **_HTML_CSS_PROPERTIES,
                "emit_pdf_path": {
                    "type": "string",
                    "description": "Optional destination PDF under the MCP workspace root; creates parent directories and replaces an existing file. Omit to return diagnostics without saving the PDF.",
                },
                "fail_on": {
                    "type": "array",
                    "items": {"enum": ["overflow", "missing-glyphs", "font-subst", "budget"]},
                    "uniqueItems": True,
                    "description": "Failure checks to run: overflow, missing-glyphs, or font-subst. Defaults to none. The budget value requires budget limits available through the CLI, which this tool does not expose.",
                },
                "allow_fallbacks": _ALLOW_FALLBACKS,
            },
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
        destructive=True,
    ),
    _tool(
        "fullbleed_inspect",
        "Inspect a PDF",
        "Read an existing PDF's version, page count, profile claims, warnings, and template-composition compatibility without changing it. Use fullbleed_verify to render and check HTML/CSS source, or fullbleed_assets with action='verify' to check asset package integrity.",
        {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Existing PDF file under the MCP workspace root, relative to the server's --root; this is a PDF path, not HTML source.",
                },
            },
            "required": ["path"],
            "additionalProperties": False,
        },
        read_only=True,
        idempotent=True,
    ),
    _tool(
        "fullbleed_assets",
        "Manage Fullbleed assets",
        "Manage font, CSS, and icon packages with list, info, install, verify, or lock. Returns package details or operation results. Install writes workspace assets and may download supported remote packages. Use fullbleed_inspect for PDFs and fullbleed_verify for document validation.",
        {
            "type": "object",
            "properties": {
                "action": {
                    "enum": ["list", "info", "install", "verify", "lock"],
                    "description": "list shows built-in and cached packages; info describes one package; install vendors it; verify checks package presence/hashes and optional lock constraints; lock creates or updates a lock file using add.",
                },
                "package": {
                    "type": "string",
                    "description": "Package name/reference, such as 'noto-sans' or '@bootstrap'. Required for info, install, and verify; ignored for list and lock. Use list to discover supported names.",
                },
                "available": {
                    "type": "boolean", "default": False,
                    "description": "For list only: include the catalog of supported remote packages in addition to built-in and cached packages. Defaults to false.",
                },
                "vendor_path": {
                    "type": "string",
                    "description": "For install only: destination directory under the MCP workspace root, default 'vendor'. Existing matching asset files may be replaced.",
                },
                "lock_path": {
                    "type": "string",
                    "description": "Lock file under the MCP workspace root. For verify it must exist; omission skips lock comparison. For lock it is created or updated and defaults to 'assets.lock.json'.",
                },
                "add": {
                    "type": "array", "items": {"type": "string"},
                    "description": 'For lock only: built-in package references to add/update, for example ["@noto-sans", "@bootstrap"]. Omit to preserve existing entries or create an empty lock file; this does not scan arbitrary project files.',
                },
                "strict": {
                    "type": "boolean", "default": False,
                    "description": "For verify only: report a lock mismatch as a tool error. Defaults to false, which returns the verification result with ok=false and violations instead.",
                },
            },
            "required": ["action"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
        open_world=True,
        destructive=True,
    ),
    _tool(
        "fullbleed_compile",
        "Compile a document family",
        "Compile inline HTML/CSS and return compile_id plus template statistics for repeated calls to fullbleed_render_compiled. Handles belong to this server process; restart or eviction after 64 retained handles requires recompiling. Use fullbleed_compile_vdp for a single variable-data job without retaining a handle.",
        {
            "type": "object",
            "properties": {
                **_COMPILED_SOURCE_PROPERTIES,
            },
            "required": ["html"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=False,
        destructive=True,
    ),
    _tool(
        "fullbleed_render_compiled",
        "Render a compiled document",
        "Write a PDF from compile_id returned by fullbleed_compile in this server session. Choose static copies, fixed geometry, or content reflow; returns the output path, page/record counts, and SHA-256. Use fullbleed_compile_vdp to compile and render a variable-data job in one call.",
        {
            "type": "object",
            "properties": {
                "compile_id": {
                    "type": "string",
                    "description": "Opaque handle returned by fullbleed_compile in this same server process. Recompile if the process restarted or the handle was evicted; it is not a filename or serialized artifact.",
                },
                "output_path": _OUTPUT_PDF_PATH,
                "mode": {
                    "enum": ["static", "fixed_bindings", "reflow_bindings"],
                    "description": "static repeats the compiled document using copies; fixed_bindings substitutes columnar text without changing geometry; reflow_bindings lays out variable-length records and repaginates. The binding modes require bindings.",
                },
                "copies": {
                    "type": "integer", "minimum": 1,
                    "description": "Number of identical document copies in static mode; defaults to 1. Ignored for binding modes, whose record count is the binding-column length.",
                },
                "bindings": _BINDINGS,
                "compression": _COMPRESSION,
            },
            "required": ["compile_id", "output_path", "mode"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
        destructive=True,
    ),
    _tool(
        "fullbleed_compile_vdp",
        "Compile and render a VDP job",
        "Compile inline HTML/CSS and render columnar variable records into one PDF, returning its path, counts, SHA-256, and compile/render metrics. Releases the temporary handle after rendering. Use fullbleed_compile plus fullbleed_render_compiled when reusing a template across calls.",
        {
            "type": "object",
            "properties": {
                **_COMPILED_SOURCE_PROPERTIES,
                "bindings": _BINDINGS,
                "mode": {
                    "enum": ["fixed_bindings", "reflow_bindings"],
                    "description": "fixed_bindings substitutes text while preserving compiled geometry; reflow_bindings recalculates layout and page count for variable-length content. Check pdf_profile_catalog.fixed_bindings_supported before combining fixed bindings with a PDF profile.",
                },
                "output_path": _OUTPUT_PDF_PATH,
                "compression": _COMPRESSION,
            },
            "required": ["html", "bindings", "mode", "output_path"],
            "additionalProperties": False,
        },
        read_only=False,
        idempotent=True,
        destructive=True,
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
                "data_bound_charts": {
                    "engine_owned": bool(
                        capabilities.get("charts", {}).get("engine_owned", False)
                    ),
                    "rust_api": capabilities.get("charts", {}).get("rust_api"),
                    "kinds": list(capabilities.get("charts", {}).get("kinds", [])),
                    "outputs": list(
                        capabilities.get("charts", {}).get("outputs", [])
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
            "- Engine-owned chart kinds: "
            + ", ".join(contract.get("capabilities", {}).get("charts", {}).get("kinds", []))
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
