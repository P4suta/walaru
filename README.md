# Walaru

An unreleased tool for local failure analysis and replay of Java and Kotlin tests.

## Installation

```sh
# Requires JDK 21 or later.
git clone https://github.com/P4suta/walaru.git
cd walaru
```

## Usage

```sh
# The zero-dependency API is fail-open without the test agent.
# CLI explain produces local analysis even when tests fail.
# Replay reports verified: true after a fresh-JVM match through the requested event.
cd examples/java-library-first
../../gradlew walaruExplain # Expected test failure; writes build/reports/walaru/index.html.
```

## Documentation

[Read the documentation](docs/README.md).

## License

Licensed under the [Apache License, Version 2.0](LICENSE-APACHE) or the [Massachusetts Institute of Technology (MIT) license](LICENSE-MIT), at your option.
