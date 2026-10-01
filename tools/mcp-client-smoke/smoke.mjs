// SPDX-License-Identifier: MIT
// Exercise the installed server through the official MCP SDK's client validators.
import assert from 'node:assert/strict';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const workspace = await mkdtemp(join(tmpdir(), 'fullbleed-sdk-smoke-'));
const client = new Client({ name: 'fullbleed-sdk-smoke', version: '1' });
const transport = new StdioClientTransport({
  command: process.argv[2] || 'python',
  args: ['-m', 'fullbleed_mcp', '--root', workspace],
  cwd: workspace,
  stderr: 'pipe',
});
let stderr = '';
const deadline = setTimeout(() => {
  console.error('MCP SDK smoke timed out after 90 seconds.');
  void transport.close().finally(() => process.exit(1));
}, 90000);
try {
  await client.connect(transport);
  transport.stderr?.on('data', chunk => { stderr += chunk; });
  // listTools validates every tool descriptor with the official SDK schema.
  const { tools } = await client.listTools();
  assert.equal(tools.length, 11);
  const capabilities = await client.callTool({ name: 'fullbleed_capabilities', arguments: {} });
  assert.equal(capabilities.structuredContent.schema, 'fullbleed.capabilities.v1');
  const preview = await client.callTool({
    name: 'fullbleed_render_preview',
    arguments: {
      html: '<h1>MCP SDK invoice</h1><p>SDK-CHECK-001</p>',
      css: '@page { size: 240pt 180pt; margin: 18pt; } body { font: 11pt Helvetica; }',
      output_dir: 'preview',
    },
  });
  assert.notEqual(preview.isError, true);
  assert.equal(preview.structuredContent.ok, true);
  const pdf = await readFile(join(workspace, 'preview/preview.pdf'));
  assert.equal(pdf.subarray(0, 5).toString(), '%PDF-');
  const failure = await client.callTool({
    name: 'fullbleed_render', arguments: { output_path: 'missing-source.pdf' },
  });
  assert.equal(failure.isError, true);
  assert.equal(failure.structuredContent.code, 'MCP_TOOL_ERROR');
  console.log(JSON.stringify({
    schema: 'fullbleed.mcp_sdk_smoke.v1', ok: true, sdk: '1.31.0',
    server: client.getServerVersion(), tool_count: tools.length,
    checks: ['tool descriptors accepted', 'structured capabilities accepted',
      'PDF preview rendered', 'structured tool error accepted'],
    workspace,
  }));
} catch (error) {
  if (stderr) console.error(stderr);
  throw error;
} finally {
  clearTimeout(deadline);
  await client.close();
}
