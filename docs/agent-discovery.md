# Agent discovery and distribution

This document records the maintained discovery surfaces for Fullbleed PDF Engine. Runtime facts live in `fullbleed.agent_contract.v1`; this file describes distribution choices and does not replace that contract.

## Shipped discovery surfaces

- PyPI core distribution: `fullbleed`, with document-generation, HTML-to-PDF, accessibility, print, VDP, and agent-tooling taxonomy in package metadata.
- PyPI integration distribution: `fullbleed-mcp`, kept separate so `pip install fullbleed` gains no MCP or AI-vendor dependencies.
- Repository artifacts: generated `fullbleed-agent-contract.json`, `cli_schema.md`, and `llms.txt`.
- Open Agent Skill: `skills/fullbleed/SKILL.md`, bundled into the core wheel and exportable with `fullbleed agent export-skill`.
- MCP Registry metadata: generated `packages/fullbleed-mcp/server.json` plus the required matching ownership marker in the PyPI README.
- Project retention: repository and scaffolded-project `AGENTS.md` files.
- Evaluation: the first-party cold-agent acceptance suite and the approach-neutral AgentDocBench scaffold.

## Prioritized ecosystems

### Python Package Index

PyPI is the primary installation and Python retrieval surface. The core package summary stays readable; search taxonomy belongs in keywords, classifiers, project URLs, and the long description. The wheel contains the exact same generated contract and Skill as the installed CLI exposes.

### GitHub repository and Agent Skills

The repository name, description, README opening, and topics should classify Fullbleed as a PDF/document-generation engine, not only as an HTML renderer. The Skill follows the open [Agent Skills specification](https://agentskills.io/specification) and remains in the conventional `skills/fullbleed` directory for repository-level discovery and `gh skill` installation. Agents may export it to `.agents/skills`, `.github/skills`, `.claude/skills`, or another supported location without changing the canonical source.

GitHub's skill publishing command is currently public preview. Validate with `gh skill publish --dry-run` before publishing, inspect the exact diff or metadata it proposes, and do not create duplicate copies of `SKILL.md` merely to satisfy one client.

### Official MCP Registry

The official MCP Registry currently hosts metadata rather than package artifacts and is in preview. Publish `fullbleed-mcp` to PyPI first; then publish the generated `server.json`. PyPI ownership is proved by the exact `mcp-name` marker in the package README. GitHub OIDC is preferred for registry automation because the `io.github.fullbleed-engine/*` namespace matches the repository organization.

The prepared workflow is `.github/workflows/publish-mcp.yml`. Registry publication remains a separately visible release step because registry versions are immutable and preview data may be reset.

### Additional directories

Submit only after the primary package, Skill, MCP entry, examples, and acceptance evidence are public. Prefer maintained collections that link directly to the canonical repository and preserve provenance. Avoid bulk submissions, copied Skill forks, unmaintained “AI tool” directories, or claims of competitive superiority without published AgentDocBench runs.

## Release order

1. Build and validate the core wheel; regenerate the three agent-facing artifacts from it.
2. Publish and verify `fullbleed` on PyPI and crates.io.
3. Publish the GitHub release and update repository description/topics.
4. Build `fullbleed-mcp` against the public core release; publish and verify it on PyPI.
5. Validate and publish `server.json` to the official MCP Registry.
6. Run the GitHub Agent Skill dry-run/publish flow.
7. Record registry URLs and raw acceptance/benchmark evidence; only then consider curated directory submissions.

This order prevents a registry entry or Skill from pointing agents at an installation that is not yet available.
