# Fullbleed Documentation

This folder documents the Fullbleed stack at three layers:

1. Engine internals and render pipeline
2. Python API surface (`import fullbleed`)
3. CLI surface (`fullbleed ...`)

## Version scope

These docs target the `2.3.1` release line and the current repository source layout. Agent-facing commands, schemas, capabilities, and profiles are generated from the installed runtime; use `fullbleed agent-contract --format json` as the source of truth.

## Documents

- `docs/install-non-technical.md`: beginner install/setup guide (Python + Fullbleed)
- `docs/css-coverage.md`: validated CSS coverage, parity status, and active gap/backlog contract
- `docs/engine.md`: Rust engine architecture, render flow, pagination model, diagnostics
- `docs/performance-architecture.md`: compiled-template, vector IR, virtualization, and shader roadmap
- `docs/performance-pass-2026-08-04.md`: subset/linker, fixed-copy, and distinct variable-data
  benchmark results and validation
- `docs/python-api.md`: Python bindings, classes, methods, and usage patterns
- `docs/ui-accessibility.md`: `fullbleed.ui` authoring, accessibility primitives, validation, signatures
- `docs/cli.md`: command reference, JSON/machine mode, reproducibility and validation flows
- `docs/agent-discovery.md`: maintained package, Skill, MCP Registry, and ecosystem distribution strategy
- `fullbleed-agent-contract.json`: generated canonical agent manifest
- `cli_schema.md`: generated human view of runtime commands, schemas, and capabilities
- `llms.txt`: generated compact discovery entrypoint for LLM retrieval
- `skills/fullbleed/SKILL.md`: first-party versionless Agent Skill
- `packages/fullbleed-mcp/README.md`: separately distributed stdio MCP adapter
- `agent_acceptance/README.md`: isolated cold-agent acceptance harness
- `examples/agent_workflows/README.md`: compact end-to-end agent-oriented examples
- `agentdocbench/README.md`: approach-neutral document-generation agent benchmark scaffold
- `docs/pdf-templates.md`: Rust finalize PDF template/XObject composition policy and smoke gates
- `docs/release/README.md`: release runbook, readiness ledger, and public claim contract
- `docs/release/2.3.1-runbook.md`: current Cargo/PyPI/GitHub release procedure
- `examples/canonical_reference/README.md`: canonical static PDF scaffold reference and validation workflow
- `examples/compiled_reflow/README.md`: compiled variable-content pagination, running strings,
  trusted structural slots, and per-call compression

## Recommended reading order

1. `docs/install-non-technical.md` if you are setting up Python and Fullbleed for the first time
2. `docs/css-coverage.md` for validated CSS coverage and known gap policy
3. `docs/python-api.md` if you are building reports/components in Python
4. `docs/ui-accessibility.md` if you are building semantic/a11y-first document workflows
5. `docs/cli.md` if you are automating builds/validation in CI
6. `skills/fullbleed/SKILL.md` if you are configuring an unfamiliar coding agent
7. `packages/fullbleed-mcp/README.md` if you need a semantic tool adapter
8. `docs/pdf-templates.md` if your workflow overlays variable data onto source PDF templates
9. `docs/performance-architecture.md` for the compiled rendering and performance contract
10. `docs/release/README.md` if you are validating, tagging, or publishing a release
11. `examples/canonical_reference/README.md` if you need a complete runnable static PDF reference project
12. `examples/compiled_reflow/README.md` for the compiled content-reflow Python lane
13. `docs/engine.md` if you need to reason about behavior, constraints, or performance

## Scaffold and component workflow

For component-first project structure and scaffold conventions, read:

- `python/fullbleed_cli/scaffold_templates/init/SCAFFOLDING.md`
- `examples/canonical_reference/SCAFFOLDING.md`
