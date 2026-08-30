package io.github.p4suta.walaru.model;

/** Kotlin- or Java-level logical source location. */
public record SourceLocation(String path, int line, int column, String symbol) {
    public SourceLocation {
        if (line < 1 || column < 1) {
            throw new IllegalArgumentException("source line and column are one-based");
        }
    }
}
