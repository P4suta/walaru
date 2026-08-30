# Replay guarantees

Walaru calls a replay exact only when the recording manifest is complete for the observed boundaries and a fresh JVM reproduces every observable event through the requested target.

Reverse navigation can select an earlier line, call, write, source location, or write watchpoint. Verification compares kind, logical location, logical thread and virtual-thread identity, coroutine stack, safely captured values, state hash, and output position. Only a match returns exit `0` with `verified: true`.

Supported deterministic inputs include selected time, random, and UUID calls. Unique logical thread names permit cooperative event-boundary scheduling. Bounded `Files.readAllBytes` and `Files.readString` inputs are opt-in through `record --capture-file-io`; public traces reveal only a redacted size.

Network access, child processes, JNI, environment/system-property boundaries, default file I/O, unsupported randomness, ambiguous thread identities, and any unreconstructable boundary make the recording partial. Partial recordings remain queryable, but replay and reverse return exit `4`.

An optional Linux recording may reference an `rr` trace and event cross-index. Walaru emits direct argv for navigation and does not claim that reaching an `rr` event proves JVM locals, fields, output, or state. CRIU and CRaC detection is informational checkpoint capability, not proof.
