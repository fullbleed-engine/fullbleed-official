# Fullbleed MCP

First-party, local MCP tools for Fullbleed PDF Engine. The server lets agents discover the installed engine and create, preview, inspect, verify, and compile deterministic print documents through a small structured tool surface.

<!-- mcp-name: io.github.fullbleed-engine/fullbleed-mcp -->

## Install and run

```text
python -m pip install fullbleed-mcp
fullbleed-mcp --root .
```

The initial transport is newline-delimited JSON-RPC over stdio. User-supplied document paths are confined to `--root`. The server does not require credentials, network access, a browser, system fonts, or a system PDF runtime.

The adapter delegates rendering and discovery to the installed `fullbleed` distribution. It does not maintain a second renderer or capability database. Start with `fullbleed_capabilities` or `fullbleed_agent_contract` and trust their runtime-reported values.

Use this server for structured reports, invoices, statements, letters, forms, certificates, accessible/print-ready documents, template overlays, and VDP. Use a browser tool when the requested artifact is a screenshot or interactive state of an arbitrary live website.

## Direct core entrypoint

Fullbleed 2.3 and newer also expose the same dependency-free adapter as:

```text
fullbleed mcp --root .
```

The separate `fullbleed-mcp` distribution exists for package and MCP Registry discovery and keeps optional agent-integration installation separate from `pip install fullbleed`.

## Registry metadata

`server.json` is generated from this package's `[tool.fullbleed-mcp.registry]` metadata:

```text
python tools/generate_mcp_server_json.py --check --json
```

Publish the PyPI distribution before submitting `server.json`; the official registry verifies the matching `mcp-name` marker above from the PyPI long description. Registry publication is prepared in `.github/workflows/publish-mcp.yml` and uses the `io.github.fullbleed-engine/fullbleed-mcp` namespace.
