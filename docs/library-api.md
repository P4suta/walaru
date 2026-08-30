# Library guide

Walaru has two library boundaries. `walaru-api` is embedded in application code to name the state a
human cares about. `walaru-client` lets Java and Kotlin tools consume the same bounded data returned
by the CLI. Neither boundary asks an application to depend on the Rust implementation.

## Instrumentation API

`io.github.p4suta.walaru.Walaru` is a zero-dependency Java 21 artifact and is directly usable from
Kotlin. Calls are observational and fail-open. When no agent is active, captures return the original
value, checkpoints and notes do nothing, spans are no-ops, and lazy diagnostics are not evaluated.

| API | Contract |
| --- | --- |
| `active()` | True only when the current thread is associated with an observed test. |
| `capture(name, value)` | Safely snapshots and returns `value` unchanged. |
| `captureRedacted(name, value)` | Records presence only; the value never reaches the backend. |
| `captureLazy(name, supplier)` | Computes diagnostic-only state only during an active test. |
| `checkpoint(name[, value])` | Adds a named source-linked boundary, optionally with safe state. |
| `note([name,] message)` | Adds a short, redacted annotation. |
| `span(name)` | Returns an `AutoCloseable` span with attributes, status, and observational duration. |
| `run` / `call` | Executes code in a span and preserves its result or original failure. |
| `context()` | Captures an opaque context for an existing executor or callback thread. |

Java:

```java
public SearchResult search(List<Item> items, String query, Executor executor) {
    try (var span = Walaru.span("catalog search")
            .capture("itemCount", items.size())
            .capture("query", query)) {
        var context = Walaru.context();
        executor.execute(context.wrap(() -> Walaru.note("cache", "refresh started")));

        int candidate = Walaru.capture("candidate", locate(items, query));
        Walaru.checkpoint("candidate selected", Map.of("index", candidate));
        return items.get(candidate);
    }
}
```

Kotlin:

```kotlin
fun median(values: List<Int>, token: String): Int = Walaru.span("median").use { span ->
    span.capture("size", values.size)
    val ordered = values.sorted()
    val middle = Walaru.capture("middle", ordered.size / 2)
    Walaru.checkpoint("ordered", ordered)
    Walaru.captureRedacted("accessToken", token)
    ordered[middle]
}
```

Newly created platform threads and virtual threads inherit the active test context. Threads in a
pool usually predate the test; wrap their `Runnable`, `Callable`, `Supplier`, or `Executor` with a
snapshot from `Walaru.context()`. Installation is scoped with `try/finally`, so a pooled thread never
retains the test after the task completes.

### Safe values

The agent recognizes primitives, strings, enums, arrays, selected JDK collections and maps, plus a
bounded field view of application objects such as records. It does not call application getters or
`toString()`. Depth, item count, and text length are bounded. Secret-looking names are redacted, and
exception messages are filtered before persistence.

Unordered JDK maps and sets are canonicalized before persistence so JVM hash salts cannot create a
false replay divergence. When an unordered container exceeds the item bound, Walaru keeps its type,
size, and truncation marker rather than persisting a nondeterministic sample.

Use `captureRedacted` for credentials regardless of their name. Use `captureLazy` for a diagnostic
projection that would be wasteful during normal execution. The supplier is not invoked without an
active test, and a runtime exception from diagnostic computation is ignored.

Span duration is stored under `observations`, not deterministic `values`, so timing differences do
not invalidate replay. Captures, checkpoints, and span state remain ordered trace events.

## Drop-in Gradle integration

The `io.github.p4suta.walaru` plugin embeds the API, fat agent, and framework listeners. It adds the
API to the main compile/runtime classpath, attaches the agent only to test JVMs, and registers:

| Task | Result |
| --- | --- |
| `test` | Normal tests plus `build/walaru/events.jsonl` and the local report. |
| `walaruVerify` | Test execution and the conservative source/dependency model used by the CLI. |
| `walaruExplain` | Verification plus an immediately readable local report. |
| `walaruReport` | Regenerates JSON, Markdown, and self-contained HTML from existing events. |
| `walaruRuntime` | Materializes the embedded API and agent without another repository lookup. |

The report lives under `build/reports/walaru/` by default. Its JSON has `schemaVersion: "1"` and a
bounded test list containing status, failure, focus, deterministic analysis, evidence, and frames.
The HTML is self-contained and escapes all observed content.

Optional configuration:

```kotlin
walaru {
    mode.set("full")
    captureFileIo.set(false)
    reportDirectory.set(layout.buildDirectory.dir("reports/test-intelligence"))
}
```

External `agentJar`, `apiJar`, event/input files, and selected-test properties exist for the Walaru
CLI and hermetic build systems. A normal project should not set them.

## Maven and custom launchers

Maven code that imports the API declares `io.github.p4suta.walaru:walaru-api`. JUnit Platform or
TestNG custom launchers add `walaru-testkit` at test runtime. The listener files use the frameworks'
standard ServiceLoader discovery; no listener class name is copied into the application.

Until artifacts are published, build them from source into an isolated local repository:

```bash
./gradlew publishToMavenLocal -Dmaven.repo.local="$PWD/.m2" --no-daemon
```

Then point Maven at `.m2` and declare the workspace version shown in `Cargo.toml`. Running
`walaru verify` or `walaru explain`
against a Maven workspace injects the agent and testkit into Surefire without modifying its POM.
Plain `mvn test` leaves the API in its intentional no-agent mode unless the user supplies a
`-javaagent` configuration.

## Typed Java/Kotlin client

`walaru-client` invokes the local executable with `ProcessBuilder`, never a shell. It concurrently
drains bounded stdout/stderr, enforces a timeout, validates envelope schema `1`, and terminates the
process tree on failure.

```java
var client = WalaruClient.builder(projectRoot)
        .binary(Path.of("/opt/walaru/bin/walaru"))
        .timeout(Duration.ofMinutes(3))
        .maxResponseBytes(1024 * 1024)
        .build();

var result = client.explain(VerifyOptions.impacted(), 5);
for (var explanation : result.envelope().data().explanations()) {
    System.out.println(explanation.analysis().summary());
    System.out.println(explanation.recording().id());
}
```

Test failures (exit `1`) and incomplete/stale evidence (exit `4`) are typed, data-bearing
`WalaruResult` values rather than exceptions. Usage, launch, timeout, response-bound, malformed JSON,
and unsupported-schema failures remain explicit. Query methods accept `WalaruQuery` for projection,
pagination, event targets, and response size.

The client covers status, doctor, tests, failure analysis, verify, one-command explain, impact,
coverage, trace, values, record, replay, reverse, and stop. Captured user values stay as Jackson
`JsonNode` because their shape is intentionally open; the surrounding protocol is strongly typed.
