package io.github.p4suta.walaru.agent;

import java.util.Map;

/** Fresh-process probe for JDK immutable collection iteration randomization. */
public final class SafeValueProcessProbe {
    private SafeValueProcessProbe() {}

    @SuppressWarnings("unchecked")
    public static void main(String[] arguments) {
        Map<String, Object> captured =
                (Map<String, Object>) SafeValue.capture(Map.of("low", 0, "high", 4));
        System.out.print(captured.get("entries"));
    }
}
