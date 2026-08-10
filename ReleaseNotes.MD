# Fullbleed Release Notes - 2.3.0

Status: release

Release date: August 10, 2026

## Summary

Fullbleed 2.3.0 makes the installed Fullbleed PDF Engine directly discoverable and usable by unfamiliar AI agents without coupling the renderer to an AI vendor. The release adds a runtime-generated semantic contract, a first-party open Agent Skill, a workspace-confined MCP adapter, project-level agent retention, actionable machine diagnostics, complete agent-oriented examples, a cold-agent acceptance harness, and the initial approach-neutral AgentDocBench infrastructure.

This is an agent-interface and distribution release, not a rendering rewrite. The deterministic engine, signed Q32.32 fixed-point geometry, exact TrueType subsetting, ordinary render path, template composition, and fixed/reflow compiled VDP behavior remain intact.

## Runtime-owned agent contract

- `fullbleed agent-contract --format json` emits `fullbleed.agent_contract.v1` from the installed binary.
- `fullbleed agent-manifest --json` and `fullbleed agent manifest --json` provide discovery aliases.
- The contract includes product identity, semantic selection boundaries, workflows, capabilities, parser-derived commands/options, result schemas, profiles, inputs/outputs, examples, limitations, MCP tools, Skill metadata, and acceptance tasks.
- `fullbleed agent-contract --format markdown|llms` generates human and retrieval-oriented views from the same payload.
- `fullbleed-agent-contract.json`, `cli_schema.md`, and `llms.txt` are regenerated from the built wheel. CI fails if their version, commands, schemas, profiles, or engine capabilities drift from the installed runtime.
- The wheel embeds the exact generated JSON contract and `llms.txt`; installed-package smoke tests compare them byte-for-byte with live runtime output.

## Agent Skill and project retention

- Ships a concise, versionless open Agent Skill at `skills/fullbleed/SKILL.md`, with focused selection, workflow, and compliance references.
- The Skill instructs agents to trust runtime introspection instead of remembered release knowledge and makes the Fullbleed/browser/general-PDF-editor boundary explicit.
- `fullbleed agent skill-path --json` locates the bundled Skill.
- `fullbleed agent export-skill|install-skill <target> --json` safely copies it only to an absent or empty target.
- `fullbleed init` now includes `AGENTS.md`; scaffold results expose artifact paths and structured next actions.
- The repository also includes contributor `AGENTS.md` guidance for preserving the dependency-free core and generated contracts.

## MCP adapter

- Adds the separately versioned `fullbleed-mcp 0.1.0` distribution so MCP discovery does not add dependencies to `pip install fullbleed`.
- Provides newline-delimited JSON-RPC 2.0 over stdio and negotiates current MCP protocol versions.
- Exposes 11 semantic, typed tools for capabilities, agent contract, project creation, rendering, previews, verification, inspection, assets, compilation, compiled rendering, and one-call fixed/reflow VDP.
- Confines user document paths to `--root`, preserves structured Fullbleed errors, reports artifacts and compiled throughput metrics, and delegates to the installed runtime rather than duplicating renderer behavior.
- The dependency-free core exposes the same implementation through `fullbleed mcp --root .`.
- Includes generated, schema-validated official MCP Registry metadata and PyPI ownership proof under `io.github.fullbleed-engine/fullbleed-mcp`.

## Agent feedback and evaluation

- High-value glyph, overflow, fallback, budget, reproducibility, scaffold, and generic CLI failures now include additive `recommended_actions`, relevant schema/capability commands, and affected values where available.
- Project scaffolds return machine-readable artifacts and next actions.
- `examples/agent_workflows` provides complete invoice, report, accessible PDF/UA, fixed VDP, and compiled-reflow workflows with preview/inspection/marker validation.
- `fullbleed agent-acceptance prepare|verify|run` supplies five isolated cold-agent tasks and an objective PDF judge without bundling a model vendor.
- AgentDocBench begins with 13 approach-neutral tasks and machine checks for artifact validity, pagination, text/order, requested profile claims, deterministic reproduction, and reported agent/setup metrics. Visual scoring remains explicitly pending rather than inferred from structural checks.

## Discoverability metadata

- PyPI and Cargo summaries now identify Fullbleed PDF Engine as a deterministic document-generation engine for reports, invoices, statements, accessibility, print output, and compiled VDP.
- PyPI keywords, classifiers, and project URLs cover PDF/document generation, HTML-to-PDF, transactional output, PDF/UA, PDF/A, PDF/X, AI agents, Agent Skills, and MCP.
- The README opening and recommendation boundary distinguish document generation from arbitrary live-website screenshots.
- Release automation prepares the independent MCP PyPI package and official preview Registry publication after the core release is public.

## Compatibility and package versions

- Adds interfaces and metadata without removing the Fullbleed 2.2 Rust, Python, CLI, template, batch, fixed-copy, fixed-binding, reflow-binding, or Stable ABI surfaces.
- Core `fullbleed` still has no third-party Python runtime dependency and adds no browser, system PDF, system font, Cairo/Pango, or AI-vendor dependency.
- Wheels retain `cp310-abi3` compatibility for CPython 3.10 through 3.14 across the 15-platform release matrix.
- Rust 1.85 remains the minimum supported toolchain.
- Packages: `fullbleed 2.3.0` on crates.io/PyPI, `fullbleed-mcp 0.1.0` on PyPI, and unchanged `fullbleed_audit_contract 0.1.3` on crates.io.

## Release gates

- Locked default and `python,svg_raster` Rust suites.
- Complete Python suite against the installed 2.3.0 ABI3 wheel.
- Installed-wheel contract/Skill/resource smoke, generated-artifact drift check, and clean MCP stdio handshake.
- MCP `server.json` validation against the current official schema and PyPI ownership-marker check.
- Canonical agent example run from a clean installed wheel.
- Skill-format validation and AgentDocBench preparation/scoring contract tests.
- Existing golden, CSS parity, license, release-metadata, package-file, MSRV consumer, and deterministic-output gates remain required.
- Golden baselines are synchronized to the public 2.2.5 wheel after the 2.2.3-era hashes were found stale; the 2.3.0 candidate and public 2.2.5 wheel produce the same example PDFs and PNGs.

Exact test counts and artifact hashes are recorded in the final 2.3.0 release run and attached distribution manifest. No competitive AgentDocBench conclusion is made in this release.
