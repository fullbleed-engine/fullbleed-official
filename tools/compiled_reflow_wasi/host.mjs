// SPDX-License-Identifier: MIT
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { WASI } from 'node:wasi';

const [modulePath, directory] = process.argv.slice(2);
if (!modulePath || !directory) throw new Error('Usage: node host.mjs module.wasm output-directory');
// Only the synthetic test directory is exposed to this probe. Deliberately ask
// for more than one thread: this host cannot satisfy that configuration.
const wasi = new WASI({ version: 'preview1', returnOnExit: true,
  args: ['compiled-reflow-probe'], env: { FULLBLEED_THREADS: '8', RAYON_NUM_THREADS: '8' },
  preopens: { '.': resolve(directory) },
});
const module = await WebAssembly.compile(await readFile(modulePath));
const instance = await WebAssembly.instantiate(module, { wasi_snapshot_preview1: wasi.wasiImport });
process.exitCode = wasi.start(instance);
