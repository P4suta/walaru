package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;

final class SafeValueTest {
    @Test
    void redactsSecretLookingStringsEvenWithoutAFieldName() {
        assertEquals("<redacted>", SafeValue.capture("fixture-secret"));
        assertEquals("<redacted>", SafeValue.capture("token=abc123"));
        assertEquals("<redacted>", SafeValue.capture("Bearer eyJhbGciOiJIUzI1NiJ9.payload"));
        assertEquals("ordinary runtime value", SafeValue.capture("ordinary runtime value"));
        assertEquals("<redacted>", SafeValue.captureNamed("apiToken", "opaque-value"));
    }

    @Test
    void neverReturnsAnUntrustedNumberThatCouldInvokeUserToStringLater() {
        HostileNumber number = new HostileNumber();

        Object captured = SafeValue.capture(number);

        assertTrue(captured instanceof Map<?, ?>);
        assertEquals(0, number.toStringCalls);
    }

    @Test
    @SuppressWarnings("unchecked")
    void capturesFieldsWithoutCallingUserCodeAndRedactsSecrets() {
        HostileValue hostile = new HostileValue();

        Object captured = SafeValue.capture(hostile);

        assertEquals(0, hostile.toStringCalls);
        assertEquals(0, hostile.getterCalls);
        Map<String, Object> object = (Map<String, Object>) captured;
        Map<String, Object> fields = (Map<String, Object>) object.get("fields");
        assertEquals("visible", fields.get("name"));
        assertEquals("<redacted>", fields.get("apiToken"));
        assertTrue(object.get("type").toString().endsWith("HostileValue"));
    }

    @Test
    @SuppressWarnings("unchecked")
    void boundsTrustedCollectionsAndMarksCyclesWithoutStringifyingElements() {
        List<Object> values = new ArrayList<>();
        HostileValue hostile = new HostileValue();
        values.add(hostile);
        values.add(values);
        for (int index = 0; index < 100; index++) values.add(index);

        Map<String, Object> captured = (Map<String, Object>) SafeValue.capture(values);
        List<Object> items = (List<Object>) captured.get("items");

        assertEquals(102, captured.get("length"));
        assertEquals(32, items.size());
        assertEquals(true, captured.get("truncated"));
        assertEquals(true, ((Map<String, Object>) items.get(1)).get("cycle"));
        assertEquals(0, hostile.toStringCalls);
        assertEquals(0, hostile.getterCalls);
    }

    private static final class HostileValue {
        private final String name = "visible";
        private final String apiToken = "must-not-escape";
        private int toStringCalls;
        private int getterCalls;

        @SuppressWarnings("unused")
        String getName() {
            getterCalls++;
            return name;
        }

        @Override
        public String toString() {
            toStringCalls++;
            return apiToken;
        }
    }

    private static final class HostileNumber extends Number {
        private static final long serialVersionUID = 1L;
        private int toStringCalls;

        @Override
        public int intValue() {
            return 7;
        }

        @Override
        public long longValue() {
            return 7;
        }

        @Override
        public float floatValue() {
            return 7;
        }

        @Override
        public double doubleValue() {
            return 7;
        }

        @Override
        public String toString() {
            toStringCalls++;
            throw new AssertionError("must not be called");
        }
    }
}
