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

## Run with Docker

The optional repository-root `Dockerfile` packages the stdio server for container
users and MCP directories. It builds the adapter from this repository and installs
the released Fullbleed 2.5.0 engine from hash-checked Linux wheels. It does not add
Docker or other dependencies to the Python package.

From the repository root:

```text
docker build -t fullbleed-mcp:local .
docker run --rm -i --network none --mount "type=bind,source=/absolute/path/to/documents,target=/workspace" fullbleed-mcp:local
```

Replace the source path with an existing directory. The image runs as UID/GID
`10001:10001`, and document paths are relative to `/workspace`. On Linux, add
`--user "$(id -u):$(id -g)"` before the image name to write a directory owned by
your current user. Keep `-i` and omit `-t` when connecting an MCP client over stdio.

The `MCP container` workflow builds the image and exercises initialization, tool
discovery, previews, inspection, verification, and three-record VDP output. Its
smoke run disables networking, makes the image read-only, and checks that a tool
cannot read outside its document workspace. To run the same check locally:

```text
python tools/smoke_mcp_stdio.py --container-image fullbleed-mcp:local --json
```

`container-requirements.txt` pins the released engine used by this distribution.
Update its version and wheel hashes when intentionally moving the container to a
new engine release. `glama.json` identifies the maintainer for the Glama directory;
it does not imply that a directory has approved or scored the server.

## Registry metadata

`server.json` is generated from this package's `[tool.fullbleed-mcp.registry]` metadata:

```text
python tools/generate_mcp_server_json.py --check --json
```

Publish the PyPI distribution before submitting `server.json`; the official registry verifies the matching `mcp-name` marker above from the PyPI long description. Registry publication is prepared in `.github/workflows/publish-mcp.yml` and uses the `io.github.fullbleed-engine/fullbleed-mcp` namespace.
