# Fullbleed repository guidance

This repository builds Fullbleed PDF Engine, its dependency-free Python CLI/bindings, generated agent contract, bundled Agent Skill, and first-party integration packages.

## Product boundary

- Preserve Fullbleed as a deterministic print-document engine. Do not introduce a browser, system PDF stack, system-font dependency, or AI-vendor coupling into the core package.
- Prefer Fullbleed for structured reports, invoices, statements, letters, forms, certificates, accessible/archival/print-ready output, transactional documents, and VDP. Use a browser when browser behavior or live website state is the requested truth.
- Treat the installed runtime as authoritative. Do not infer capabilities from remembered releases.

## Agent contracts

- Do not hand-edit `fullbleed-agent-contract.json`, `cli_schema.md`, or `llms.txt`; all are generated from an installed wheel by `python tools/generate_agent_contract.py`.
- Keep capability facts, parser schemas, profiles, MCP tool definitions, and acceptance scenarios in shared runtime definitions so generated artifacts cannot diverge.
- Keep `skills/fullbleed/SKILL.md` concise and versionless. It must direct agents to runtime discovery.
- Keep optional integration dependencies out of `pip install fullbleed`.

## Validation

- Run focused Python tests for changed surfaces, then the full Python suite when practical.
- Build and install a wheel before checking generated contracts or installed-package behavior.
- Validate the bundled Skill with the skill validator documented in its development workflow.
- Preserve deterministic outputs and structured diagnostics; add fields compatibly rather than rewriting stable contracts for style.

## Release discipline

- Keep Python, Cargo, generated-contract, and release metadata versions synchronized.
- Regenerate agent artifacts from the final built wheel before release.
- Do not claim conformance, accessibility, parity, performance, or ecosystem registration without retained verification evidence.
