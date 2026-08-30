package io.github.p4suta.walaru.agent;

import java.io.File;
import java.net.URI;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;

record AgentConfiguration(List<Path> roots, AgentMode mode) {
    static AgentConfiguration fromProperties() {
        return new AgentConfiguration(
                parseRoots(System.getProperty("walaru.classRoots", "")),
                AgentMode.parse(System.getProperty("walaru.mode", "fast")));
    }

    boolean includes(Path location) {
        if (roots.isEmpty()) return false;
        Path normalized = location.toAbsolutePath().normalize();
        return roots.stream().anyMatch(root -> normalized.startsWith(root) || root.startsWith(normalized));
    }

    private static List<Path> parseRoots(String value) {
        if (value.isBlank()) return List.of();
        return Arrays.stream(value.split(java.util.regex.Pattern.quote(File.pathSeparator)))
                .filter(part -> !part.isBlank())
                .map(AgentConfiguration::path)
                .map(candidate -> candidate.toAbsolutePath().normalize())
                .toList();
    }

    private static Path path(String value) {
        return value.startsWith("file:") ? Path.of(URI.create(value)) : Path.of(value);
    }
}
