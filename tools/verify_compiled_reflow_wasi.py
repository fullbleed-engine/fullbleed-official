"""Build and execute the same compiled-reflow fixture natively and on WASIp1.

Requires Rust with wasm32-wasip1 installed and Node >=22. Uses no npm packages,
browser, system fonts, or system PDF renderer. Output must be a new directory.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--cargo', default='cargo')
    parser.add_argument('--node', default='node')
    parser.add_argument('--toolchain')
    parser.add_argument('--target-dir', type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    fixture = repo / 'tools/compiled_reflow_wasi'
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    project = out / 'probe'
    project.mkdir()
    env = {key: value for key, value in os.environ.items()
           if key not in ('RUST_MIN_STACK', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR')}
    if args.target_dir:
        env['CARGO_TARGET_DIR'] = str(args.target_dir.resolve())
    env['CARGO_BUILD_JOBS'] = '2'
    env['CARGO_INCREMENTAL'] = '0'
    env['CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS'] = '-C link-arg=--max-memory=536870912'
    cargo = [args.cargo, *(['+' + args.toolchain] if args.toolchain else [])]
    report = dict(status='starting', startedAt=datetime.now(timezone.utc).isoformat(), steps=[],
                  sourceSha256={name: sha256((repo / name).read_bytes()).hexdigest() for name in (
                      'src/lib.rs', 'src/parallel.rs', 'src/pdf.rs', 'src/compiled_report.rs', 'Cargo.toml', 'Cargo.lock',
                      'tools/compiled_reflow_wasi/probe.rs', 'tools/compiled_reflow_wasi/host.mjs')})

    def save() -> None:
        (out / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')

    def run(name: str, command: list[str], *, cwd: Path = repo, extra_env: dict | None = None) -> None:
        report['status'] = name
        save()
        print(name + ': running', flush=True)
        log = out / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(command, cwd=cwd, env={**env, **(extra_env or {})}, stdout=stream, stderr=subprocess.STDOUT)
        report['steps'].append(dict(name=name, code=result.returncode, logSha256=sha256(log.read_bytes()).hexdigest()))
        save()
        if result.returncode:
            print('\n'.join(log.read_text(encoding='utf-8', errors='replace').splitlines()[-45:]), flush=True)
            raise RuntimeError(name + ' failed')
        print(name + ': passed', flush=True)

    try:
        # Reuse the checkout's locked dependencies; adding only a test binary
        # must not silently select newer registry packages.
        base = json.loads(subprocess.check_output(cargo + ['metadata', '--locked', '--format-version', '1'], cwd=repo, env=env))
        registry = lambda data: sorted((p['name'], p['version'], p['source']) for p in data['packages'] if p['source'])
        shutil.copyfile(repo / 'Cargo.lock', project / 'Cargo.lock')
        shutil.copyfile(fixture / 'probe.rs', project / 'main.rs')
        (project / 'Cargo.toml').write_text(
            '[package]\nname = "fullbleed-compiled-reflow-probe"\nversion = "0.0.0"\nedition = "2024"\n'
            '\n[[bin]]\nname = "fullbleed-compiled-reflow-probe"\npath = "main.rs"\n'
            '\n[dependencies]\nfullbleed = { path = ' + json.dumps(repo.as_posix()) + ' }\n'
            '\n[profile.release]\nopt-level = "s"\nlto = true\nstrip = true\npanic = "abort"\n', encoding='utf-8')
        metadata = json.loads(subprocess.check_output(cargo + ['metadata', '--offline', '--format-version', '1'], cwd=project, env=env))
        assert registry(metadata) == registry(base), 'Probe changed registry dependencies'
        target = Path(metadata['target_directory'])
        report['registryDependencies'] = registry(base)
        report['probeLockSha256'] = sha256((project / 'Cargo.lock').read_bytes()).hexdigest()
        run('build-native', cargo + ['build', '--release', '--locked'], cwd=project)
        native = target / 'release' / ('fullbleed-compiled-reflow-probe.exe' if sys.platform == 'win32' else 'fullbleed-compiled-reflow-probe')
        run('build-wasi', cargo + ['build', '--release', '--locked', '--target', 'wasm32-wasip1'], cwd=project)
        wasm = target / 'wasm32-wasip1/release/fullbleed-compiled-reflow-probe.wasm'
        report['binaries'] = {name: dict(bytes=p.stat().st_size, sha256=sha256(p.read_bytes()).hexdigest())
                              for name, p in [('native', native), ('wasi', wasm)]}
        for mode, threads in [('native-1', 1), ('native-4', 4), ('wasi-8', 8)]:
            directory = out / mode
            directory.mkdir()
            shutil.copyfile(repo / 'python/fullbleed_assets/fonts/NotoSans-Regular.ttf', directory / 'NotoSans-Regular.ttf')
            command = [args.node, str(fixture / 'host.mjs'), str(wasm), str(directory)] if mode.startswith('wasi') else [str(native)]
            run(mode, command, cwd=directory, extra_env={'FULLBLEED_THREADS': str(threads), 'RAYON_NUM_THREADS': str(threads)})
        manifests = {}
        for mode in ('native-1', 'native-4', 'wasi-8'):
            directory = out / mode
            result = json.loads((directory / 'verification.json').read_bytes())
            assert len(result['cases']) == 4 and result['replayChecks'] == 4 and result['recoveryPages'] == 1
            assert result['diagnosticCalls'] == 10 and result['diagnosticFailuresRecovered'] is True
            assert all(result[key] is True for key in ('preCancelledRejected', 'unequalColumnsRejected', 'invalidFragmentRejected'))
            manifests[mode] = {p.name: dict(bytes=p.stat().st_size, sha256=sha256(p.read_bytes()).hexdigest())
                               for p in sorted(directory.iterdir()) if p.suffix in ('.pdf', '.png', '.json')}
        assert manifests['native-1'] == manifests['native-4'] == manifests['wasi-8'], 'Native/WASI output differs'
        for name, expected in report['sourceSha256'].items():
            assert sha256((repo / name).read_bytes()).hexdigest() == expected, 'Source changed during verification: ' + name
        report.update(status='passed', outputs=manifests, exactComparedFiles=len(manifests['native-1']),
                      scope='Four reflow fixture PDFs and finalized previews, two missing-character diagnostic PDFs, ten byte-preserving report calls, and replay/cancellation/error checks. No general parity, conformance, throughput, or Node API claim.')
    except BaseException as error:
        report.update(status='failed', error=str(error))
        raise
    finally:
        report['finishedAt'] = datetime.now(timezone.utc).isoformat()
        save()
    print(json.dumps(dict(status=report['status'], exactComparedFiles=report['exactComparedFiles'])), flush=True)


if __name__ == '__main__':
    main()
