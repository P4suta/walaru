# Public contracts

## CLI and envelope

Lifecycle commands are `status`, `watch`, `tui`, `cancel`, `stop`, and `doctor`. Execution uses `verify`, `explain`, `record`, `replay`, and `reverse`; queries use `tests`, `failure`, `impact`, `coverage`, `trace`, and `values`.

`explain` is the human-default compound operation: it verifies the requested revision, loads each
bounded structured failure, performs deterministic offline analysis, and records up to the requested
number of failed tests in full. A test failure remains exit `1`; analysis and recording IDs are still
usable data. Full recordings share a two-minute wall-clock budget; completed explanations are
preserved, omitted failures are counted, and budget exhaustion is reported in both data and
diagnostics. It never calls a remote model.

If compilation or build configuration fails before a framework emits a test failure, `explain`
returns `buildFailure` with a bounded summary, the local worker-log path, and next steps instead of
pretending that test evidence exists.

Finite commands accept `--format`, `--fields`, `--limit`, `--cursor`, `--max-bytes`, and `--at`. Field masks are relative to `data`. Trace pagination accounts for the projected event size, so excluding a large `values` field does not reduce the requested item count. The final encoded envelope still obeys `--max-bytes`.

Envelope schema version `1` fixes:

```text
schemaVersion workspaceId revision sessionId runId status data
diagnostics capabilities nextActions page
```

`tests[*].lastFailureId` is nullable and opens the latest structured failure without another search. `nextActions[*].argv` is an argument vector, never a shell string. JSON and NDJSON are stable machine formats; human output and the TUI are presentation layers. The canonical schema is [`schemas/envelope-v1.schema.json`](../schemas/envelope-v1.schema.json).

`failure.data.analysis` and `explain.data.explanations[*].analysis` contain a bounded summary, likely
cause, optional source focus, already-redacted evidence, and ordered suggestions. This is
deterministic evidence classification, not a claim of certainty. Additive payload fields do not
change envelope schema version `1`.

## Live verification

`verify --overlay-manifest PATH --supersede` is the editor-neutral live boundary. The versioned
manifest contains complete unsaved UTF-8 documents; its canonical schema is
[`schemas/overlay-manifest-v1.schema.json`](../schemas/overlay-manifest-v1.schema.json). Paths are
canonical workspace-relative `/` paths. Requests are limited to 256 documents, 1 MiB per document,
and 4 MiB total.

Overlays execute in a private persistent mirror and never write the real workspace. The mirror
retains Gradle state, restores files when an editor buffer becomes clean, and hashes source inputs
so equal-length edits cannot be missed. `--test` may be repeated for exact public test IDs. Without
an explicit selection, the normal conservative impact policy applies.

A superseding request cancels the active worker and serializes replacement work. The superseded
request exits `4` with `status: "stale"`, `data.cancelled: true`, and diagnostic
`WALARU_SUPERSEDED`. `cancel` is idempotent. Successful and failed verification payloads include
`overlayVersions`, source-linked `problems`, `testStatuses`, and `valueHints`. A value hint links a
safe `capture`, `checkpoint`, `note`, `spanValue`, or non-empty line value to its test, event, path,
and line. Each run returns at most 256 hints and replaces a value larger than 4 KiB with an explicit
placeholder. `evidenceFormatVersion` lets the daemon reject an older successful cache when newly
required evidence is unavailable. Clients must publish a result only if its document versions and
local request generation are still current.

## Library events and reports

The public API emits `capture`, `checkpoint`, `note`, `spanStart`, `spanValue`, and `spanEnd` event
kinds. Deterministic `values` participate in replay comparison; non-deterministic `observations`
(currently span duration) do not. `captureRedacted` persists only `<redacted>` and never passes the
original value across the API bridge.

Gradle reports are separate local artifacts under `build/reports/walaru`: `report.json` has report
schema version `1`, while `report.md` and `index.html` are presentation formats. The report reader
bounds individual lines, total events, evidence count, rendered values, stack frames, and escaped
HTML. Report schema versioning is independent of the CLI envelope version.

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
