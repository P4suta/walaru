# Kotlin example

From `examples/kotlin-library-first`, run:

```bash
../../gradlew walaruExplain
```

The median implementation intentionally fails. The same zero-dependency Java API works with Kotlin
`use`, generics, collections, and named assertions. Open `build/reports/walaru/index.html`, or build
the CLI. Then, from the repository root, run:

```bash
target/debug/walaru --workspace examples/kotlin-library-first --format human explain
```
