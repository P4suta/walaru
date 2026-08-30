# Architecture

Walaru binds each observation to a canonical hash of source, resource, Maven/Gradle configuration, and wrapper inputs. One worktree owns one daemon and SQLite WAL store under `.gradle/walaru/<workspace-id>`.

The Rust CLI sends bounded, versioned protobuf requests over a local Unix socket or Windows loopback endpoint. Responses use the public JSON envelope. SQLite stores query metadata directly and zstd-compresses events and recordings; orphaned runs are recovered as errors.

The Gradle init script or Maven Surefire launch injects a Java ASM agent without editing the target project. Fast verification records deduplicated coverage and dependency evidence. Full recording captures ordered execution, safe values, output positions, supported deterministic inputs, and capability boundaries.

Successful runs persist a revision manifest. Known implementation edits use learned edges; unknown dependencies, public surface changes, resources, and build inputs widen conservatively. A changed worktree can never produce a fresh success for an earlier revision.

Replay is a fresh execution, not in-place heap undo. The JVM backend selects an earlier event and verifies the observable prefix. The optional Linux `rr` boundary can create an argv navigation plan, but navigation alone is not exact JVM state proof.

The TUI, VS Code extension, IntelliJ configuration, and agent skill are thin clients of the same bounded API. They do not own execution or replay state.
