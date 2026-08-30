package io.github.p4suta.walaru.agent;

import io.github.p4suta.walaru.model.KotlinSourceMap;

/** Agent-local facade over the public compiler-independent source-map contract. */
final class SourceMap {
    private final KotlinSourceMap delegate;

    private SourceMap(KotlinSourceMap delegate) {
        this.delegate = delegate;
    }

    static SourceMap parse(String debug, String defaultSource) {
        return new SourceMap(KotlinSourceMap.parse(debug, defaultSource));
    }

    Position position(int outputLine) {
        KotlinSourceMap.Position position = delegate.position(outputLine);
        return new Position(position.path(), position.line());
    }

    int map(int outputLine) {
        return position(outputLine).line;
    }

    String sourcePath() {
        return delegate.sourcePath();
    }

    record Position(String path, int line) {}
}
