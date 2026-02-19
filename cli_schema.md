<!-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial -->
# CLI JSON Contract (v1)

Machine contract for `fullbleed` automation.

## Invocation Mode

- Use `--json` or `--json-only`.
- `--json-only` is preferred for agents.

## Exit Codes

- `0`: success.
- `1`: command-level validation/operational failure.
- `2`: argparse usage error (usage text, not JSON).
- `3`: CLI runtime/input error wrapper.

## Parsing Rules

1. Check process exit code first.
2. If exit code is `0`, parse JSON payload.
3. If exit code is `2`, parse stderr/stdout as usage text (do not expect JSON).
4. If exit code is `1` or `3`, attempt JSON parse first; if parse fails, treat stdout/stderr as text diagnostics.

## Common Payload Fields

- `schema`: stable schema id string (for example `fullbleed.render_result.v1`).
- `ok`: boolean success indicator (present on command/error payloads).
- `code`: machine error code on failure payloads.
- `message`: human-readable detail on failure payloads.

## Key Result Shapes

### RenderResult

```json
{
  "schema": "fullbleed.render_result.v1",
  "ok": true,
  "bytes_written": 41398,
  "outputs": {
    "pdf": "out/report.pdf",
    "jit": "out/jit.jsonl",
    "perf": "out/perf.jsonl",
    "glyph_report": null,
    "page_data": null,
    "compose_plan": null,
    "sha256": "3c8f27e6...",
    "artifact_sha256": "8b1f6d8c...",
    "image_sha256": [
      "b9b19826...",
      "6f1e5f09..."
    ],
    "deterministic_hash": "out/report.sha256",
    "deterministic_hash_sha256": "8b1f6d8c...",
    "deterministic_hash_mode": "artifact_set_v1"
  }
}
```

### VerifyResult

```json
{
  "schema": "fullbleed.verify_result.v1",
  "ok": true,
  "bytes_written": 0,
  "outputs": {
    "pdf": null,
    "jit": "out/jit.jsonl",
    "perf": "out/perf.jsonl",
    "glyph_report": null,
    "page_data": null,
    "compose_plan": null
  }
}
```

### AssetsInstallResult

```json
{
  "schema": "fullbleed.assets_install.v1",
  "ok": true,
  "name": "bootstrap",
  "version": "5.0.0",
  "installed_to": "C:\\Users\\...\\fullbleed\\cache\\packages\\bootstrap\\5.0.0\\bootstrap.min.css",
  "sha256": "3c8f27e6...",
  "license": "MIT",
  "license_file": "C:\\Users\\...\\LICENSE.bootstrap.txt",
  "source": "builtin",
  "install_scope": "global_cache",
  "project_detected": false
}
```

Compatibility note:
- `install_scope` and `project_detected` may be absent in older builds.

### ComplianceResult

```json
{
  "schema": "fullbleed.compliance.v1",
  "ok": true,
  "license": {
    "spdx_expression": "AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial",
    "mode": "commercial",
    "commercial": {
      "attested": true,
      "license_id": "ACME-2026-001"
    }
  },
  "policy": {
    "schema": "fullbleed.cli_compliance.v1",
    "package_license": "AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial"
  },
  "flags": []
}
```

Commercial attestation options for `compliance`:

- `--license-mode auto|agpl|commercial`
- `--commercial-licensed`
- `--commercial-license-id <id>`
- `--commercial-license-file <path>`

### CapabilitiesResult

```json
{
  "schema": "fullbleed.capabilities.v1",
  "commands": ["render", "verify", "plan", "run", "finalize", "inspect", "compliance", "capabilities"],
  "agent_flags": ["--json", "--json-only", "--schema", "--emit-manifest", "--emit-compose-plan"],
  "engine": {
    "batch_render": true,
    "batch_render_parallel": true,
    "glyph_report": true,
    "page_data": true,
    "pdf_inspect": true,
    "template_catalog_inspect": true,
    "template_compose_planner": true
  },
  "svg": {
    "document_input": {
      "html_file_accepts_svg": true,
      "html_str_accepts_svg_markup": true,
      "inline_svg_in_html": true
    },
    "asset_bundle": {
      "kind": "svg",
      "auto_kind_from_extension": true
    },
    "engine_flags": {
      "svg_form_xobjects": true,
      "svg_raster_fallback": true
    }
  }
}
```

### InspectPdfResult

```json
{
  "schema": "fullbleed.inspect_pdf.v1",
  "ok": true,
  "path": "input/sample.pdf",
  "pdf_version": "1.7",
  "page_count": 1,
  "encrypted": false,
  "file_size_bytes": 218734,
  "warnings": [],
  "composition": {
    "supported": true,
    "issues": []
  }
}
```

### InspectTemplatesResult

```json
{
  "schema": "fullbleed.inspect_templates.v1",
  "ok": true,
  "templates": [
    {
      "template_id": "sample-template-a",
      "path": "templates/template_a.pdf",
      "page_count": 2,
      "composition": {"supported": true, "issues": []}
    }
  ],
  "metrics": {
    "templates": 1,
    "compatible_templates": 1,
    "incompatible_templates": 0
  }
}
```

### InspectPdfBatchResult

```json
{
  "schema": "fullbleed.inspect_pdf_batch.v1",
  "ok": false,
  "items": [
    {"path": "input/a.pdf", "ok": true, "page_count": 2},
    {"path": "input/missing.pdf", "ok": false, "code": "PDF_NOT_FOUND"}
  ],
  "metrics": {"total": 2, "ok": 1, "failed": 1}
}
```

SVG notes:

- `render --html file.svg` is a valid direct SVG-document path.
- `render --html-str "<svg ...>"` is valid for inline SVG markup.
- `--asset <path.svg>` infers `asset_kind=svg` when omitted.

### ErrorResult

```json
{
  "schema": "fullbleed.error.v1",
  "ok": false,
  "code": "CLI_ERROR",
  "message": "human readable error message"
}
```

## Schema Discovery

Use runtime schema discovery for exact command contracts:

```bash
fullbleed --schema render
fullbleed --schema verify
fullbleed --schema assets install
fullbleed --schema assets verify
fullbleed --schema capabilities
fullbleed --schema inspect pdf
fullbleed --schema inspect templates
fullbleed --schema inspect pdf-batch
```

## Known Schema IDs (Primary)

- `fullbleed.render_result.v1`
- `fullbleed.verify_result.v1`
- `fullbleed.plan_result.v1`
- `fullbleed.run_result.v1`
- `fullbleed.compliance.v1`
- `fullbleed.doctor.v1`
- `fullbleed.capabilities.v1`
- `fullbleed.assets_list.v1`
- `fullbleed.assets_info.v1`
- `fullbleed.assets_install.v1`
- `fullbleed.assets_verify.v1`
- `fullbleed.assets_lock.v1`
- `fullbleed.cache_dir.v1`
- `fullbleed.cache_prune.v1`
- `fullbleed.init.v1`
- `fullbleed.new_template.v1`
- `fullbleed.new_list.v1`
- `fullbleed.new_search.v1`
- `fullbleed.new_remote.v1`
- `fullbleed.inspect_pdf.v1`
- `fullbleed.inspect_templates.v1`
- `fullbleed.inspect_pdf_batch.v1`
- `fullbleed.compose_plan.v1`
- `fullbleed.error.v1`

