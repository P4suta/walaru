# Java example

Run:

```bash
../../gradlew walaruExplain
```

The binary search intentionally fails. Walaru returns the original Gradle failure status and writes
`build/reports/walaru/index.html`, `report.md`, and `report.json`. The report links the failure to the
last partition, shows named values, and contains `<redacted>` rather than the API token.

For a full ordered recording after building the CLI, run from the repository root:

```bash
target/debug/walaru --workspace examples/java-library-first --format human explain
```
