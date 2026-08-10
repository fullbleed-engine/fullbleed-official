# AgentDocBench

AgentDocBench is an open, approach-neutral benchmark scaffold for document-generation agents. It measures whether an agent can turn realistic structured requests into validated PDF artifacts; it does not require Fullbleed-specific authoring or APIs.

The initial task set covers reports, invoices, letters, certificates, accessible and archival/print-ready documents, existing PDF templates, multilingual content, overflow repair, deterministic reproduction, high-volume personalized output, and content that changes pagination.

## Prepare isolated tasks

```text
python tools/agentdocbench.py prepare --workspace output/agentdocbench --json
```

Each task directory contains only `TASK.json`, structured inputs, and an explicit fixture when required. Point any document-generation agent or adapter at that directory. The expected PDF path is declared in the task.

## Submission metadata

An agent may write `submission.json` beside its output:

```json
{
  "schema": "agentdocbench.submission.v1",
  "approach": "tool-or-stack-name",
  "agent": "model-and-agent-name",
  "first_pass_success": true,
  "metrics": {
    "agent_tokens": 0,
    "tool_calls": 0,
    "correction_loops": 0,
    "setup_failures": 0,
    "execution_ms": 0
  }
}
```

Unknown or unavailable metrics should be omitted, not estimated.

The scorer aggregates reported tokens, tool calls, correction loops, setup failures, execution time, and first-pass outcomes separately from artifact correctness. Submission metrics are self-reported in this initial scaffold; comparative runs should use an external harness to capture them independently.

## Score machine-checkable dimensions

```text
python tools/agentdocbench.py score --workspace output/agentdocbench --json
```

The reference judge checks deliverables, page ranges, extracted markers/order, standards claims where the task explicitly requests them, and byte identity for the deterministic-reproduction task. It records visual review as pending; the initial infrastructure does not pretend that structural checks establish visual correctness.

The reference inspector happens to use Fullbleed's PDF inspection/text-extraction surface because it is available in this repository, but submissions may be produced by any engine. Future adapters can replace or cross-check the judge without changing task semantics.

Do not publish competitive conclusions from this initial suite. First collect multi-approach runs, validate the rubric against human review, publish raw artifacts/metrics, and disclose environment and model variance.
