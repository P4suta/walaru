# Architecture

Walaru binds each observation to a canonical hash of source, resource, Maven/Gradle configuration, and wrapper inputs. One worktree owns one daemon and SQLite WAL store under `.gradle/walaru/<workspace-id>`.

The library boundary is deliberately smaller than the runtime. `walaru-api` has no dependencies and
uses an optional method-handle bridge, so production code remains runnable without Walaru. The
Gradle plugin embeds that API, the agent, and framework listeners. Maven and custom launchers use the
same artifacts separately. Named captures, checkpoints, notes, spans, and propagated async contexts
become ordinary ordered events; runtime-only measurements are kept as non-deterministic
`observations`.

The Rust CLI sends bounded, versioned protobuf requests over a local Unix socket or Windows loopback endpoint. Unix worktrees whose state path exceeds portable socket limits use a workspace-ID endpoint under `/tmp`, still mode `0600`; requests remain bound to the canonical worktree. Responses use the public JSON envelope. SQLite stores query metadata directly and zstd-compresses events and recordings; orphaned runs are recovered as errors.

The drop-in Gradle plugin, Gradle init script, or Maven Surefire launch injects a Java ASM agent. Fast verification records deduplicated coverage and dependency evidence. Full recording captures ordered execution, safe values, output positions, supported deterministic inputs, and capability boundaries. JUnit Platform and TestNG lifecycle adapters are discovered through ServiceLoader.

Successful runs persist a revision manifest. Known implementation edits use learned edges; unknown dependencies, public surface changes, resources, and build inputs widen conservatively. A changed worktree can never produce a fresh success for an earlier revision.

Replay is a fresh execution, not in-place heap undo. The JVM backend selects an earlier event and verifies the observable prefix. The optional Linux `rr` boundary can create an argv navigation plan, but navigation alone is not exact JVM state proof.

The generated Gradle report and deterministic failure analyzer make the library useful without any
editor or AI client. The typed JVM client, CLI/TUI, VS Code extension, IntelliJ configuration, and
agent skill are additional clients of the same bounded API; none owns execution or replay state.
