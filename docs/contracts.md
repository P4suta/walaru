# Public contracts

## CLI and envelope

Lifecycle commands are `status`, `watch`, `tui`, `stop`, and `doctor`. Execution uses `verify`, `record`, `replay`, and `reverse`; queries use `tests`, `failure`, `impact`, `coverage`, `trace`, and `values`.

Finite commands accept `--format`, `--fields`, `--limit`, `--cursor`, `--max-bytes`, and `--at`. Field masks are relative to `data`. Trace pagination accounts for the projected event size, so excluding a large `values` field does not reduce the requested item count. The final encoded envelope still obeys `--max-bytes`.

Envelope schema version `1` fixes:

```text
schemaVersion workspaceId revision sessionId runId status data
diagnostics capabilities nextActions page
```

`tests[*].lastFailureId` is nullable and opens the latest structured failure without another search. `nextActions[*].argv` is an argument vector, never a shell string. JSON and NDJSON are stable machine formats; human output and the TUI are presentation layers. The canonical schema is [`schemas/envelope-v1.schema.json`](../schemas/envelope-v1.schema.json).

## Exit codes

- `0`: operation completed.
- `1`: compilation or selected tests failed.
- `2`: usage or configuration error.
- `3`: daemon, worker, or internal error.
- `4`: stale revision, response limit, partial/unsupported capability, or unverified replay.

Exit `4` must never be reported as exact success.

## Bounds and identity

RPC frames are at most 16 MiB. Queries clamp to 1,000 items and 4 KiB–1 MiB response budgets. A default response is at most 65,536 bytes. Worker event lines are at most 1 MiB, runs at most one million events, and worker log tails at most 8 MiB.

Workspace, revision, event, failure, run, and recording IDs are stable within their documented inputs. Fresh replay IDs differ and are compared by ordered observable content, not run-local identifiers.
