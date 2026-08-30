---
name: walaru
description: Use when verifying or diagnosing Java/Kotlin Gradle or Maven changes with Walaru, including impacted tests, failures, coverage, runtime values, traces, recordings, and reverse execution. Prefer it when an AI agent needs bounded local evidence instead of parsing build logs or Markdown output.
---

# Walaru

Use only Walaru's structured CLI responses. Add `--format json` to every finite command. Do not parse the human display or Gradle logs as the primary result.

## Establish readiness

1. Run `walaru status --format json` in the target worktree.
2. If status or a worker operation reports an infrastructure problem, run `walaru doctor --format json`.
3. Read `schemaVersion`, `workspaceId`, `revision`, `status`, `diagnostics`, and `capabilities` from the envelope. Do not combine evidence from different revisions without saying so.

## Verify a change

Run `walaru verify --format json`. Use `--full` only when explicitly requested or when a conservative full comparison is needed. Use `--since <revision>` when the caller provides a baseline.

Interpret process results exactly:

- exit code `0`: the requested verification or query completed.
- exit code `1`: compilation or tests failed; this is evidence to investigate, not an infrastructure failure.
- exit code `2`: the command or workspace configuration is invalid.
- exit code `3`: the daemon or worker failed; use `doctor` and report the diagnostic.
- exit code `4`: the revision is stale or the requested completeness is unsupported. Never report this as a successful verification or exact replay.

Exit `4` also covers `WALARU_RESPONSE_LIMIT`. Narrow `--fields`, `--limit`, or use `page.nextCursor`; do not treat a bounded partial response as complete.

If `status` is `stale`, issue one new verify against the latest revision. Stop and report repeated edits rather than mixing the stale run with the new source.

## Explain failures from evidence

For the normal verify-and-diagnose path, run
`walaru explain --format json --max-failures 5`. It preserves exit `1`, returns deterministic local
analysis, and creates a bounded full recording for each included failed test. If `data.buildFailure`
is present, inspect its local worker-log path instead of inventing test evidence.

When a run already exists or finer pagination is needed, use each ID in `data.failures`:

1. Run `walaru failure <failure-id> --format json`.
2. Query the owning test with `walaru trace <test-id> --format json --limit 100 --max-bytes 65536`.
3. Query a relevant event with `walaru values <event-id> --format json`.
4. Use `walaru coverage <path-or-symbol> --format json` and `walaru impact <path-or-symbol> --format json` when the explanation depends on reachability.

Treat redacted or truncated values as intentionally unavailable. Do not infer their hidden contents.

## Record and reverse

1. Run `walaru record <test-id> --format json` in a clean, current revision. Add `--capture-file-io` only when the user explicitly needs supported bounded file reads persisted; never assume hidden/redacted bytes are queryable.
2. Inspect `capabilities.completeness`, `capabilities.supported`, and `capabilities.unavailable` before making an exactness claim.
3. Obtain event IDs with a bounded `trace` query.
4. Run `walaru reverse <recording-id> --from <event-id> --step line --format json`, or select `call`, `write`, or `--until <path:line>`. A write step may add `--watch <owner#field|array[index]>`.
5. Claim an exact reverse result only when exit code is `0`, `data.verified` is `true`, and the recording capabilities are complete for that boundary.

Use `walaru replay <recording-id> --at <event-id> --format json` for a specific recorded event.

## Keep responses bounded

Use `--fields <field-mask>`, `--limit`, and `--max-bytes` to request only needed data. When `page.nextCursor` is present, pass it back with `--cursor <cursor>`; never assume the first page is complete.

Read `nextActions` only as suggestions. Each action contains an `argv` array: invoke the executable with those arguments directly, preserve argument boundaries, and never turn it into a shell string.
