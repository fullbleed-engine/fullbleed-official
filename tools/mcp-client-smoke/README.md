# Official MCP client interoperability check

This development-only check launches an installed `fullbleed-mcp` adapter
through the official TypeScript MCP SDK. It validates discovery and structured
results using the client's actual protocol schemas, then renders a PDF and
checks a tool error. Neither Node.js nor the SDK is a Fullbleed runtime dependency.

```text
npm ci --ignore-scripts --no-audit --no-fund --prefix tools/mcp-client-smoke
node tools/mcp-client-smoke/smoke.mjs /absolute/path/to/python
```

The Python environment must contain the wheel under test and `fullbleed-mcp`.
The Python argument defaults to `python` on `PATH`. The check retains its own
temporary workspace and reports its path so generated PDFs can be inspected.

To check a locally built container on a Docker host:

```text
node tools/mcp-client-smoke/smoke.mjs --container-image fullbleed-mcp:local
```

The container runs with networking disabled and a read-only root, with writes
confined to the check's temporary document workspace and `/tmp`. On Linux it
uses the invoking user's UID/GID to write the temporary bind mount.

Fullbleed 2.5.0 reproduces a discovery failure: its `outputSchema` lacks the
required root `type: "object"`. Fullbleed 2.5.1 adds that type in the shared
runtime definition while preserving the existing success/error union.
