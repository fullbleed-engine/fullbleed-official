# Agent-oriented document workflows

These versionless examples are compact, complete workflows intended for humans and coding agents to discover and copy. They use only the installed Fullbleed runtime and standard library:

```text
structured input
→ HTML/CSS authoring
→ PDF render
→ PNG preview where appropriate
→ PDF inspection and text validation
→ machine-readable result
```

Run all five canonical cases from an installed wheel:

```text
python examples/agent_workflows/run_examples.py --out output/agent-examples --json
```

The cases are:

- a one-page invoice from JSON-like structured data;
- a naturally paginated business report;
- a tagged PDF/UA document using the canonically bundled Inter font;
- a 100-record fixed-geometry compiled VDP job;
- a variable-length compiled reflow VDP job.

Every case inspects the resulting PDF and verifies required text markers. Ordinary documents also emit PNG page previews. The runner returns `fullbleed.agent_examples_result.v1` and exits nonzero on validation failure.

For an existing-PDF template composition workflow, use `examples/template-flagging-smoke/`. For a larger annotated reflow case study, use `examples/compiled_reflow/`. Always consult `fullbleed agent-contract --format json` before assuming the installed version exposes those surfaces.
