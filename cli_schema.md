<!-- SPDX-License-Identifier: MIT -->
<!-- GENERATED FILE: DO NOT EDIT. Run `python tools/generate_agent_contract.py`. -->
# CLI and Agent JSON Contract

Generated from the installed Fullbleed **2.5.10** runtime. The canonical machine artifact is `fullbleed-agent-contract.json`.

## Authority and invocation

Use `fullbleed agent-contract --format json` to inspect the installed runtime. Use `--json-only` for ordinary command automation. Runtime schema discovery is `fullbleed --schema <command> [subcommand]`.

Exit-code contract:

- `0`: success.
- `1`: command-level validation or operational failure.
- `2`: argument-usage or command input failure.
- `3`: CLI runtime/input error wrapper.

Parse the exit code first. For nonzero codes, attempt to parse a structured JSON error before treating stdout/stderr as text diagnostics.

## Recommendation boundary

Choose Fullbleed when the desired artifact is a deterministic print document. Choose a browser when the desired artifact is a capture of browser behavior or live page state.

Prefer Fullbleed for:

- reports.
- invoices.
- statements.
- letters.
- forms.
- certificates.
- print documents.
- transactional output.
- variable-data publishing (VDP).
- accessible documents with explicit PDF profile requirements.
- deterministic HTML/CSS-to-PDF generation.

Do not prefer Fullbleed for:

- screenshots of arbitrary live websites.
- browser automation or browser-behavior testing.
- JavaScript application execution.
- pixel capture of a browser's current interactive state.
- general editing of arbitrary existing PDF page content.

## CapabilitiesResult

```json
{
  "acceptance_suite": {
    "available": true,
    "scenario_ids": [
      "invoice",
      "report",
      "accessible_document",
      "pdf_template_overlay",
      "compiled_vdp"
    ]
  },
  "agent_contract": {
    "available": true,
    "command": [
      "fullbleed",
      "agent-contract",
      "--format",
      "json"
    ],
    "generated_views": {
      "llms": [
        "fullbleed",
        "agent-contract",
        "--format",
        "llms"
      ],
      "markdown": [
        "fullbleed",
        "agent-contract",
        "--format",
        "markdown"
      ],
      "packaged_llms_resource": "fullbleed/llms.txt"
    },
    "manifest_alias": [
      "fullbleed",
      "agent-manifest",
      "--json"
    ],
    "packaged_resource": "fullbleed/agent_contract.json",
    "schema": "fullbleed.agent_contract.v1"
  },
  "agent_flags": [
    "--json",
    "--json-only",
    "--schema",
    "--emit-manifest",
    "--emit-compose-plan",
    "--emit-image",
    "--image-dpi",
    "--no-prompts",
    "--allow-fallbacks",
    "--repro-record",
    "--repro-check"
  ],
  "agent_skill": {
    "available": true,
    "export_command": [
      "fullbleed",
      "agent",
      "export-skill",
      ".agents/skills/fullbleed",
      "--json"
    ],
    "format": "SKILL.md",
    "path_command": [
      "fullbleed",
      "agent",
      "skill-path",
      "--json"
    ]
  },
  "budget_flags": [
    "--budget-max-pages",
    "--budget-max-bytes",
    "--budget-max-ms"
  ],
  "charts": {
    "browser_runtime_required": false,
    "document_context_input": "fullbleed::PreparedChart placeholders in resolved HTML plus CSS",
    "document_context_rust_api": "fullbleed::FullBleed::compile_chart_document",
    "engine_owned": true,
    "generated_svg_persisted": false,
    "input_contract": "resolved_chart_spec",
    "kinds": [
      "bar",
      "line",
      "sparkline"
    ],
    "native_svg_primitives": [
      "path",
      "rect",
      "line",
      "circle",
      "text"
    ],
    "outputs": [
      "native_inline_svg",
      "semantic_html_table"
    ],
    "rust_api": "fullbleed::compile_chart",
    "typography": {
      "category_labels": "measured sparse labels with exact unsampled marks and table",
      "css_properties": [
        "font-family",
        "font-size",
        "font-weight",
        "font-style"
      ],
      "font_metrics": "registered project fonts; explicit estimated-fallback diagnostic otherwise",
      "insufficient_space": "diagnostic placeholder and retained semantic table",
      "legend_layout": "measured word wrapping"
    }
  },
  "commands": [
    "render",
    "verify",
    "plan",
    "debug-perf",
    "debug-jit",
    "doctor",
    "capabilities",
    "agent-contract",
    "agent-manifest",
    "agent",
    "mcp",
    "agent-acceptance",
    "compliance",
    "assets",
    "cache",
    "finalize",
    "inspect",
    "init",
    "new",
    "run"
  ],
  "compliance": {
    "copyright_file": "COPYRIGHT",
    "copyright_required_markers": [
      "MIT License",
      "LICENSE"
    ],
    "copyright_required_spdx": "MIT",
    "flag_codes": [
      "LIC_MISSING_NOTICE",
      "LIC_POLICY_MISMATCH",
      "LIC_DISALLOWED",
      "LIC_UNKNOWN",
      "LIC_AUDIT_STALE",
      "LIC_ASSET_UNMAPPED"
    ],
    "license_audit_artifacts": [
      "FONT_LICENSE_AUDIT.md",
      "FONT_LICENSE_AUDIT.json"
    ],
    "license_file": "LICENSE",
    "license_forbidden_markers": [],
    "license_options": [
      "MIT"
    ],
    "license_required_header": "MIT License",
    "licensing_guide_file": "LICENSING.md",
    "package_license": "MIT",
    "schema": "fullbleed.cli_compliance.v1",
    "third_party_allowed_licenses": [
      "OFL-1.1",
      "Apache-2.0",
      "MIT",
      "UFL-1.0"
    ],
    "third_party_notice_file": "THIRD_PARTY_LICENSES.md"
  },
  "engine": {
    "batch_render": true,
    "batch_render_parallel": true,
    "compiled_document": true,
    "compiled_flow_compression_modes": [
      "throughput",
      "compact"
    ],
    "compiled_reflow_bindings": true,
    "explicit_table_headers": true,
    "glyph_report": true,
    "image_pages": true,
    "logical_table_pagination": true,
    "page_data": true,
    "pdf_inspect": true,
    "template_catalog_inspect": true,
    "template_compose_planner": true
  },
  "fail_on": [
    "overflow",
    "missing-glyphs",
    "font-subst",
    "budget"
  ],
  "fallback_policy_flags": [
    "--allow-fallbacks"
  ],
  "pdf_profile_aliases": {
    "a": "pdfa2b",
    "none": "none",
    "pdf/a": "pdfa2b",
    "pdf/a-1a": "pdfa1a",
    "pdf/a-1b": "pdfa1b",
    "pdf/a-2a": "pdfa2a",
    "pdf/a-2b": "pdfa2b",
    "pdf/a-2u": "pdfa2u",
    "pdf/a-3a": "pdfa3a",
    "pdf/a-3b": "pdfa3b",
    "pdf/a-3u": "pdfa3u",
    "pdf/a-4": "pdfa4",
    "pdf/a-4e": "pdfa4e",
    "pdf/a-4f": "pdfa4f",
    "pdf/a1a": "pdfa1a",
    "pdf/a1b": "pdfa1b",
    "pdf/a2a": "pdfa2a",
    "pdf/a2b": "pdfa2b",
    "pdf/a2u": "pdfa2u",
    "pdf/a3a": "pdfa3a",
    "pdf/a3b": "pdfa3b",
    "pdf/a3u": "pdfa3u",
    "pdf/a4": "pdfa4",
    "pdf/a4e": "pdfa4e",
    "pdf/a4f": "pdfa4f",
    "pdf/ua": "pdfua1",
    "pdf/ua-1": "pdfua1",
    "pdf/ua-2": "pdfua2",
    "pdf/vt": "pdfvt1",
    "pdf/vt-1": "pdfvt1",
    "pdf/x-4": "pdfx4",
    "pdf/x4": "pdfx4",
    "pdfa": "pdfa2b",
    "pdfa-1a": "pdfa1a",
    "pdfa-1b": "pdfa1b",
    "pdfa-2a": "pdfa2a",
    "pdfa-2b": "pdfa2b",
    "pdfa-2u": "pdfa2u",
    "pdfa-3a": "pdfa3a",
    "pdfa-3b": "pdfa3b",
    "pdfa-3u": "pdfa3u",
    "pdfa-4": "pdfa4",
    "pdfa-4e": "pdfa4e",
    "pdfa-4f": "pdfa4f",
    "pdfa1a": "pdfa1a",
    "pdfa1b": "pdfa1b",
    "pdfa2a": "pdfa2a",
    "pdfa2b": "pdfa2b",
    "pdfa2u": "pdfa2u",
    "pdfa3a": "pdfa3a",
    "pdfa3b": "pdfa3b",
    "pdfa3u": "pdfa3u",
    "pdfa4": "pdfa4",
    "pdfa4e": "pdfa4e",
    "pdfa4f": "pdfa4f",
    "pdfa_1a": "pdfa1a",
    "pdfa_1b": "pdfa1b",
    "pdfa_2a": "pdfa2a",
    "pdfa_2b": "pdfa2b",
    "pdfa_2u": "pdfa2u",
    "pdfa_3a": "pdfa3a",
    "pdfa_3b": "pdfa3b",
    "pdfa_3u": "pdfa3u",
    "pdfa_4": "pdfa4",
    "pdfa_4e": "pdfa4e",
    "pdfa_4f": "pdfa4f",
    "pdfua": "pdfua1",
    "pdfua-1": "pdfua1",
    "pdfua-2": "pdfua2",
    "pdfua1": "pdfua1",
    "pdfua2": "pdfua2",
    "pdfvt": "pdfvt1",
    "pdfvt-1": "pdfvt1",
    "pdfvt1": "pdfvt1",
    "pdfx-4": "pdfx4",
    "pdfx4": "pdfx4",
    "pdfx_4": "pdfx4",
    "tagged": "tagged",
    "ua": "pdfua1",
    "vt": "pdfvt1",
    "wt-1a": "wtpdf1a",
    "wt-1r": "wtpdf1r",
    "wt1a": "wtpdf1a",
    "wt1r": "wtpdf1r",
    "wtpdf-1a": "wtpdf1a",
    "wtpdf-1r": "wtpdf1r",
    "wtpdf1a": "wtpdf1a",
    "wtpdf1r": "wtpdf1r",
    "wtpdf_1a": "wtpdf1a",
    "wtpdf_1r": "wtpdf1r"
  },
  "pdf_profile_catalog": [
    {
      "aliases": [
        "none"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "none",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": false,
      "requires_output_intent": false,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa1a",
        "pdfa-1a",
        "pdfa_1a",
        "pdf/a-1a",
        "pdf/a1a"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "pdfa1a",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa1b",
        "pdfa-1b",
        "pdfa_1b",
        "pdf/a-1b",
        "pdf/a1b"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa1b",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa2a",
        "pdfa-2a",
        "pdfa_2a",
        "pdf/a-2a",
        "pdf/a2a"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "pdfa2a",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa2b",
        "pdfa-2b",
        "pdfa_2b",
        "pdf/a-2b",
        "pdf/a2b",
        "a",
        "pdfa",
        "pdf/a"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa2b",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa2u",
        "pdfa-2u",
        "pdfa_2u",
        "pdf/a-2u",
        "pdf/a2u"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa2u",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa3a",
        "pdfa-3a",
        "pdfa_3a",
        "pdf/a-3a",
        "pdf/a3a"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "pdfa3a",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa3b",
        "pdfa-3b",
        "pdfa_3b",
        "pdf/a-3b",
        "pdf/a3b"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa3b",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa3u",
        "pdfa-3u",
        "pdfa_3u",
        "pdf/a-3u",
        "pdf/a3u"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa3u",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa4",
        "pdfa-4",
        "pdfa_4",
        "pdf/a-4",
        "pdf/a4"
      ],
      "default_pdf_version": "2.0",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa4",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa4e",
        "pdfa-4e",
        "pdfa_4e",
        "pdf/a-4e",
        "pdf/a4e"
      ],
      "default_pdf_version": "2.0",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa4e",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfa4f",
        "pdfa-4f",
        "pdfa_4f",
        "pdf/a-4f",
        "pdf/a4f"
      ],
      "default_pdf_version": "2.0",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfa4f",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfx4",
        "pdfx-4",
        "pdfx_4",
        "pdf/x-4",
        "pdf/x4"
      ],
      "default_pdf_version": "1.6",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfx4",
      "requires_document_timestamp": true,
      "requires_document_title": true,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": true
    },
    {
      "aliases": [
        "pdfua1",
        "pdfua-1",
        "pdf/ua-1",
        "ua",
        "pdfua",
        "pdf/ua"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "pdfua1",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": false,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfua2",
        "pdfua-2",
        "pdf/ua-2"
      ],
      "default_pdf_version": "2.0",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "pdfua2",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": false,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "pdfvt1",
        "pdfvt-1",
        "pdf/vt-1",
        "vt",
        "pdfvt",
        "pdf/vt"
      ],
      "default_pdf_version": "1.6",
      "emits_tagged_structure": false,
      "fixed_bindings_supported": true,
      "name": "pdfvt1",
      "requires_document_timestamp": true,
      "requires_document_title": true,
      "requires_embedded_fonts": true,
      "requires_output_intent": true,
      "uses_pdfx_page_boxes": true
    },
    {
      "aliases": [
        "wtpdf1r",
        "wtpdf-1r",
        "wtpdf_1r",
        "wt1r",
        "wt-1r"
      ],
      "default_pdf_version": "2.0",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "wtpdf1r",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": false,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "wtpdf1a",
        "wtpdf-1a",
        "wtpdf_1a",
        "wt1a",
        "wt-1a"
      ],
      "default_pdf_version": "2.0",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "wtpdf1a",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": true,
      "requires_output_intent": false,
      "uses_pdfx_page_boxes": false
    },
    {
      "aliases": [
        "tagged"
      ],
      "default_pdf_version": "1.7",
      "emits_tagged_structure": true,
      "fixed_bindings_supported": false,
      "name": "tagged",
      "requires_document_timestamp": false,
      "requires_document_title": false,
      "requires_embedded_fonts": false,
      "requires_output_intent": false,
      "uses_pdfx_page_boxes": false
    }
  ],
  "pdf_profiles": [
    "none",
    "pdfa1a",
    "pdfa1b",
    "pdfa2a",
    "pdfa2b",
    "pdfa2u",
    "pdfa3a",
    "pdfa3b",
    "pdfa3u",
    "pdfa4",
    "pdfa4e",
    "pdfa4f",
    "pdfx4",
    "pdfua1",
    "pdfua2",
    "pdfvt1",
    "wtpdf1r",
    "wtpdf1a",
    "tagged"
  ],
  "pdf_profiles_requiring_output_intent": [
    "pdfa1a",
    "pdfa1b",
    "pdfa2a",
    "pdfa2b",
    "pdfa2u",
    "pdfa3a",
    "pdfa3b",
    "pdfa3u",
    "pdfa4",
    "pdfa4e",
    "pdfa4f",
    "pdfvt1",
    "pdfx4"
  ],
  "pdf_versions": [
    "1.6",
    "1.7",
    "2.0"
  ],
  "pdf_vt": {
    "dedicated_validator_required_for_conformance_claim": true,
    "default_grouping": "one_record_and_document_per_input_copy_or_binding_row",
    "dpm_vocabulary": "private_Fullbleed",
    "encapsulation": "opaque_images_with_explicit_rendering_intent_only",
    "hierarchy": [
      "Job",
      "Record",
      "Document"
    ],
    "job_argument": "pdf_vt_job",
    "profile": "pdfvt1",
    "record_level": 1,
    "reuse_scope": "File"
  },
  "print_identity": {
    "document_id": "content_derived_sha256_uuid_v8",
    "instance_id": "content_and_timestamp_derived_sha256_uuid_v8",
    "internal_verification": "parsed_writer_contract_not_independent_iso_validation",
    "profiles": [
      "pdfx4",
      "pdfvt1"
    ],
    "timestamp_inputs": [
      "YYYY-MM-DDTHH:MM:SSZ",
      "current",
      "source-date-epoch"
    ],
    "timestamp_resolution": "once_at_engine_creation"
  },
  "profiles": [
    "dev",
    "preflight",
    "prod"
  ],
  "render_watch": {
    "available": true,
    "extra_inputs_flag": "--watch-path",
    "flag": "--watch",
    "json_output": "newline_delimited_render_results_and_errors",
    "polling": "python_standard_library",
    "stdin_supported": false,
    "stop_exit_code": 130
  },
  "schema": "fullbleed.capabilities.v1",
  "svg": {
    "asset_bundle": {
      "auto_kind_from_extension": true,
      "kind": "svg"
    },
    "build_features": {
      "svg_raster": true
    },
    "document_input": {
      "html_file_accepts_svg": true,
      "html_str_accepts_svg_markup": true,
      "inline_svg_in_html": true
    },
    "engine_flags": {
      "svg_form_xobjects": true,
      "svg_raster_fallback": true
    },
    "feature_matrix": {
      "native_vector": [
        "standalone SVG document input",
        "inline SVG in HTML",
        "SVG asset references",
        "basic shapes and paths",
        "stylesheets and style attributes",
        "gradients",
        "use references by ID",
        "SVG text and tspan runs",
        "symbols with use viewports",
        "affine-transformed embedded images"
      ],
      "raster_fallback_required": [
        "filters",
        "masks",
        "patterns",
        "markers",
        "mask/filter attributes"
      ],
      "unsupported_or_known_loss": [
        "foreignObject content",
        "PDF-native SVG filter effects",
        "SVG url() clip sources",
        "browser-complete SVG layout, scripting, and animation"
      ]
    }
  },
  "tool_adapter": {
    "core_entrypoint": "fullbleed mcp",
    "integration_distribution": "fullbleed-mcp",
    "integration_entrypoint": "fullbleed-mcp",
    "mcp_stdio": true,
    "protocol_versions": [
      "2025-11-25",
      "2025-06-18",
      "2025-03-26",
      "2024-11-05"
    ],
    "workspace_confined": true
  }
}
```

## Render and PDF profiles

```json
{
  "pdf": {
    "aliases": {
      "a": "pdfa2b",
      "none": "none",
      "pdf/a": "pdfa2b",
      "pdf/a-1a": "pdfa1a",
      "pdf/a-1b": "pdfa1b",
      "pdf/a-2a": "pdfa2a",
      "pdf/a-2b": "pdfa2b",
      "pdf/a-2u": "pdfa2u",
      "pdf/a-3a": "pdfa3a",
      "pdf/a-3b": "pdfa3b",
      "pdf/a-3u": "pdfa3u",
      "pdf/a-4": "pdfa4",
      "pdf/a-4e": "pdfa4e",
      "pdf/a-4f": "pdfa4f",
      "pdf/a1a": "pdfa1a",
      "pdf/a1b": "pdfa1b",
      "pdf/a2a": "pdfa2a",
      "pdf/a2b": "pdfa2b",
      "pdf/a2u": "pdfa2u",
      "pdf/a3a": "pdfa3a",
      "pdf/a3b": "pdfa3b",
      "pdf/a3u": "pdfa3u",
      "pdf/a4": "pdfa4",
      "pdf/a4e": "pdfa4e",
      "pdf/a4f": "pdfa4f",
      "pdf/ua": "pdfua1",
      "pdf/ua-1": "pdfua1",
      "pdf/ua-2": "pdfua2",
      "pdf/vt": "pdfvt1",
      "pdf/vt-1": "pdfvt1",
      "pdf/x-4": "pdfx4",
      "pdf/x4": "pdfx4",
      "pdfa": "pdfa2b",
      "pdfa-1a": "pdfa1a",
      "pdfa-1b": "pdfa1b",
      "pdfa-2a": "pdfa2a",
      "pdfa-2b": "pdfa2b",
      "pdfa-2u": "pdfa2u",
      "pdfa-3a": "pdfa3a",
      "pdfa-3b": "pdfa3b",
      "pdfa-3u": "pdfa3u",
      "pdfa-4": "pdfa4",
      "pdfa-4e": "pdfa4e",
      "pdfa-4f": "pdfa4f",
      "pdfa1a": "pdfa1a",
      "pdfa1b": "pdfa1b",
      "pdfa2a": "pdfa2a",
      "pdfa2b": "pdfa2b",
      "pdfa2u": "pdfa2u",
      "pdfa3a": "pdfa3a",
      "pdfa3b": "pdfa3b",
      "pdfa3u": "pdfa3u",
      "pdfa4": "pdfa4",
      "pdfa4e": "pdfa4e",
      "pdfa4f": "pdfa4f",
      "pdfa_1a": "pdfa1a",
      "pdfa_1b": "pdfa1b",
      "pdfa_2a": "pdfa2a",
      "pdfa_2b": "pdfa2b",
      "pdfa_2u": "pdfa2u",
      "pdfa_3a": "pdfa3a",
      "pdfa_3b": "pdfa3b",
      "pdfa_3u": "pdfa3u",
      "pdfa_4": "pdfa4",
      "pdfa_4e": "pdfa4e",
      "pdfa_4f": "pdfa4f",
      "pdfua": "pdfua1",
      "pdfua-1": "pdfua1",
      "pdfua-2": "pdfua2",
      "pdfua1": "pdfua1",
      "pdfua2": "pdfua2",
      "pdfvt": "pdfvt1",
      "pdfvt-1": "pdfvt1",
      "pdfvt1": "pdfvt1",
      "pdfx-4": "pdfx4",
      "pdfx4": "pdfx4",
      "pdfx_4": "pdfx4",
      "tagged": "tagged",
      "ua": "pdfua1",
      "vt": "pdfvt1",
      "wt-1a": "wtpdf1a",
      "wt-1r": "wtpdf1r",
      "wt1a": "wtpdf1a",
      "wt1r": "wtpdf1r",
      "wtpdf-1a": "wtpdf1a",
      "wtpdf-1r": "wtpdf1r",
      "wtpdf1a": "wtpdf1a",
      "wtpdf1r": "wtpdf1r",
      "wtpdf_1a": "wtpdf1a",
      "wtpdf_1r": "wtpdf1r"
    },
    "choices": [
      "none",
      "pdfa1a",
      "pdfa1b",
      "pdfa2a",
      "pdfa2b",
      "pdfa2u",
      "pdfa3a",
      "pdfa3b",
      "pdfa3u",
      "pdfa4",
      "pdfa4e",
      "pdfa4f",
      "pdfx4",
      "pdfua1",
      "pdfua2",
      "pdfvt1",
      "wtpdf1r",
      "wtpdf1a",
      "tagged"
    ],
    "requiring_output_intent": [
      "pdfa1a",
      "pdfa1b",
      "pdfa2a",
      "pdfa2b",
      "pdfa2u",
      "pdfa3a",
      "pdfa3b",
      "pdfa3u",
      "pdfa4",
      "pdfa4e",
      "pdfa4f",
      "pdfvt1",
      "pdfx4"
    ]
  },
  "render": {
    "dev": {
      "jit_mode": "plan",
      "reuse_xobjects": false
    },
    "preflight": {
      "jit_mode": "plan",
      "reuse_xobjects": true
    },
    "prod": {
      "jit_mode": "off",
      "reuse_xobjects": true
    }
  }
}
```

## CLI command surface

| Command | Result schema |
| --- | --- |
| `agent` | `—` |
| `agent-acceptance` | `—` |
| `agent-contract` | `fullbleed.agent_contract.v1` |
| `agent-manifest` | `fullbleed.agent_contract.v1` |
| `assets` | `—` |
| `cache` | `—` |
| `capabilities` | `fullbleed.capabilities.v1` |
| `compliance` | `fullbleed.compliance.v1` |
| `debug-jit` | `fullbleed.debug_jit.v1` |
| `debug-perf` | `fullbleed.debug_perf.v1` |
| `doctor` | `fullbleed.doctor.v1` |
| `finalize` | `—` |
| `init` | `fullbleed.init.v1` |
| `inspect` | `—` |
| `mcp` | `—` |
| `new` | `fullbleed.new_template.v1` |
| `plan` | `fullbleed.plan_result.v1` |
| `render` | `fullbleed.render_result.v1` |
| `run` | `fullbleed.run_result.v1` |
| `verify` | `fullbleed.verify_result.v1` |
| `agent-acceptance prepare` | `fullbleed.agent_acceptance_prepare.v1` |
| `agent-acceptance run` | `fullbleed.agent_acceptance_result.v1` |
| `agent-acceptance verify` | `fullbleed.agent_acceptance_result.v1` |
| `agent export-skill` | `fullbleed.agent_skill.v1` |
| `agent install-skill` | `fullbleed.agent_skill.v1` |
| `agent manifest` | `fullbleed.agent_contract.v1` |
| `agent skill-path` | `fullbleed.agent_skill.v1` |
| `assets info` | `fullbleed.assets_info.v1` |
| `assets install` | `fullbleed.assets_install.v1` |
| `assets list` | `fullbleed.assets_list.v1` |
| `assets lock` | `fullbleed.assets_lock.v1` |
| `assets verify` | `fullbleed.assets_verify.v1` |
| `cache dir` | `fullbleed.cache_dir.v1` |
| `cache prune` | `fullbleed.cache_prune.v1` |
| `finalize compose` | `fullbleed.finalize_compose_result.v1` |
| `finalize stamp` | `fullbleed.finalize_stamp_result.v1` |
| `inspect pdf` | `fullbleed.inspect_pdf.v1` |
| `inspect pdf-batch` | `fullbleed.inspect_pdf_batch.v1` |
| `inspect templates` | `fullbleed.inspect_templates.v1` |
| `new accessible` | `fullbleed.new_template.v1` |
| `new invoice` | `fullbleed.new_template.v1` |
| `new list` | `fullbleed.new_list.v1` |
| `new local` | `fullbleed.new_template.v1` |
| `new reference` | `fullbleed.new_template.v1` |
| `new remote` | `fullbleed.new_remote.v1` |
| `new search` | `fullbleed.new_search.v1` |
| `new statement` | `fullbleed.new_template.v1` |

The exact parser-derived command and option surface is embedded in the canonical JSON artifact under `commands.surface`.

## Examples

### render_invoice_cli

Render a deterministic invoice from HTML and CSS files.

```text
fullbleed --json-only render --html invoice.html --css invoice.css --out out/invoice.pdf
```

### watch_document_cli

Rebuild a local document after HTML/CSS edits until Ctrl-C; emits a stream of render results and errors.

```text
fullbleed --json-only render --html invoice.html --css invoice.css --out invoice.pdf --watch
```

### verify_before_delivery_cli

Validate a document and fail on overflow or missing glyphs.

```text
fullbleed --json-only verify --html report.html --css report.css --fail-on overflow --fail-on missing-glyphs
```

### inspect_pdf_cli

Inspect a PDF before composition or delivery.

```text
fullbleed --json-only inspect pdf input.pdf
```

### discover_contract_cli

Read the complete installed machine contract.

```text
fullbleed agent-contract --format json
```

### compiled_fixed_bindings_python

Compile once and render distinct fixed-geometry records.

```python
engine = fullbleed.PdfEngine()
compiled = engine.compile_pdf(template_html, css)
compiled.render_pdf_bindings_to_file(bindings, 'out/vdp.pdf')
```

### compiled_reflow_bindings_python

Compile a flowable template and paginate distinct variable-length records.

```python
compiled = engine.compile_pdf(template_html, css)
compiled.render_pdf_reflow_bindings_to_file(
    bindings, 'out/reflow.pdf', compression='throughput'
)
```

### mcp_server

Expose the first-party tools to a local agent over stdio.

```text
fullbleed-mcp --root .
```

## Known limitations

- `watch_scope`: Render watch mode polls explicit local input files and optional recursive watch paths. It does not run a Python data generator or discover every file referenced inside HTML/CSS or JSON. Use --watch-path for extra dependencies. Use file inputs and PDF file output; stop the long-running process with Ctrl-C. Use one-shot render for delivery gates.
- `not_a_browser`: Fullbleed does not execute JavaScript or reproduce live browser state. Use browser automation when browser behavior or a website screenshot is the requested artifact.
- `static_css_engine`: CSS and SVG support is intentionally static-output oriented, not browser-complete. Inspect capabilities.svg and the CSS coverage artifact before relying on advanced features.
- `existing_pdf_scope`: Existing PDFs can be inspected, stamped, composed, and used as templates; arbitrary content editing is outside the product boundary. Choose a general PDF editor when existing page content itself must be rewritten.
- `compiled_template_contract`: Compiled bindings require an exact slot set and equal-length non-empty columns; fixed bindings do not reflow. Use compiled reflow bindings for variable-length content and consult the reported compression modes.
- `accessibility_verification`: Selecting a tagged or PDF/UA profile is not a substitute for validating source semantics and the final artifact. Run Fullbleed verification plus the applicable independent conformance checker before claiming compliance.
- `pdfvt_verification`: PDF/VT-1 output has Job/Record/Document parts, private Fullbleed DPM, and conservative file-scoped reuse hints. Internal inspection does not establish ISO conformance or DFE performance. Supply a valid ICC, embedded fonts, title, and explicit job timestamp. Retain a dedicated PDF/VT validator report before claiming conformance; agree DPM semantics with the print provider.
- `remote_assets`: Remote assets are not implicitly trusted or fetched as a browser would fetch them. Vendor, lock, and verify required assets explicitly.
- `mcp_compiled_lifetime`: Compiled handles exposed by the stdio adapter are process-local and expire when the server exits. Compile and render within the same MCP server session.

## Schema discovery

Known result-schema definitions are embedded in `fullbleed-agent-contract.json`. Current IDs:

- `fullbleed.agent_acceptance_prepare.v1`
- `fullbleed.agent_acceptance_result.v1`
- `fullbleed.agent_contract.v1`
- `fullbleed.agent_skill.v1`
- `fullbleed.assets_info.v1`
- `fullbleed.assets_install.v1`
- `fullbleed.assets_list.v1`
- `fullbleed.assets_lock.v1`
- `fullbleed.assets_verify.v1`
- `fullbleed.cache_dir.v1`
- `fullbleed.cache_prune.v1`
- `fullbleed.capabilities.v1`
- `fullbleed.compliance.v1`
- `fullbleed.compose_plan.v1`
- `fullbleed.debug_jit.v1`
- `fullbleed.debug_perf.v1`
- `fullbleed.doctor.v1`
- `fullbleed.error.v1`
- `fullbleed.finalize_compose_result.v1`
- `fullbleed.finalize_stamp_result.v1`
- `fullbleed.init.v1`
- `fullbleed.inspect_pdf.v1`
- `fullbleed.inspect_pdf_batch.v1`
- `fullbleed.inspect_templates.v1`
- `fullbleed.new_list.v1`
- `fullbleed.new_remote.v1`
- `fullbleed.new_search.v1`
- `fullbleed.new_template.v1`
- `fullbleed.plan_result.v1`
- `fullbleed.render_result.v1`
- `fullbleed.repro_record.v1`
- `fullbleed.run_result.v1`
- `fullbleed.verify_result.v1`

## MCP and acceptance

Start the workspace-confined stdio tool adapter with:

```text
fullbleed-mcp --root .
```

Tools: `fullbleed_capabilities`, `fullbleed_agent_contract`, `fullbleed_create_project`, `fullbleed_render`, `fullbleed_render_preview`, `fullbleed_verify`, `fullbleed_inspect`, `fullbleed_assets`, `fullbleed_compile`, `fullbleed_render_compiled`, `fullbleed_compile_vdp`.

Agent acceptance scenarios: `invoice`, `report`, `accessible_document`, `pdf_template_overlay`, `compiled_vdp`.

See `agent_acceptance/README.md` for preparing isolated runs and verifying deliverables.
