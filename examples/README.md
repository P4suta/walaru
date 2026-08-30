# Library-first examples

Both examples apply one Walaru Gradle plugin from this source checkout. The plugin supplies the
zero-dependency API, Java agent, JUnit/TestNG lifecycle adapter, and local JSON/Markdown/HTML report.
No Walaru dependency or test listener is declared by the examples.

- [`java-library-first`](java-library-first) demonstrates named values, a checkpoint, a redacted
  value, and a span around an intentionally broken binary search.
- [`kotlin-library-first`](kotlin-library-first) uses the same Java API naturally from Kotlin around
  an intentionally broken median calculation.

Run either example from its directory with `../../gradlew walaruExplain`. The expected build result
is a test failure; open `build/reports/walaru/index.html` for the useful result.
