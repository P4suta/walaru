---
date: 2026-11-23
status: accepted
---

# Preserve timestamp precision in the SQLite store

## Context

The store writes UTC timestamps as RFC 3339 text and compares the values during pruning.
The public run records use Chrono, whose maintenance has ended.

## Decision

Expose a run timestamp backed by the existing `time` dependency.
Keep the UTC offset spelling, fractional precision, and leap-second values already accepted from stored runs.
Use `time::SignedDuration` for retention intervals.

## Alternatives

A plain UTC date-time loses the distinction between an inserted second and an ordinary second.
An integer column would require existing workspaces to migrate their databases.

## Consequences

Rust callers construct `RunTimestamp` values for run start and finish times.
Existing SQLite rows keep their format and databases open without a migration.

## Confirmation

Read existing timestamps, write them back, and compare the stored text.
Test pruning immediately before, at, and after a retention boundary.
