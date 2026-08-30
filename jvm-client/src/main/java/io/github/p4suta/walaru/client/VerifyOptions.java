package io.github.p4suta.walaru.client;

/** Conservative verification selection. */
public record VerifyOptions(boolean full, String since) {
    public VerifyOptions {
        if (full && since != null && !since.isBlank()) {
            throw new IllegalArgumentException("full and since are mutually exclusive");
        }
    }

    public static VerifyOptions impacted() {
        return new VerifyOptions(false, null);
    }

    public static VerifyOptions fullWorkspace() {
        return new VerifyOptions(true, null);
    }

    public static VerifyOptions since(String revision) {
        return new VerifyOptions(false, revision);
    }
}
