package io.github.p4suta.walaru.client;

import java.util.List;

/** Bounded projection and pagination options for query operations. */
public record WalaruQuery(
        int limit, String cursor, int maxBytes, String at, List<String> fields) {
    public WalaruQuery {
        limit = Math.clamp(limit, 1, 1_000);
        maxBytes = Math.clamp(maxBytes, 4_096, 1024 * 1024);
        fields = fields == null ? List.of() : List.copyOf(fields);
    }

    public static WalaruQuery defaults() {
        return new WalaruQuery(100, null, 65_536, null, List.of());
    }

    public WalaruQuery next(String nextCursor) {
        return new WalaruQuery(limit, nextCursor, maxBytes, at, fields);
    }

    public WalaruQuery projecting(String... requestedFields) {
        return new WalaruQuery(limit, cursor, maxBytes, at, List.of(requestedFields));
    }
}
