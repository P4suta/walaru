package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
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

    @Test
    @SuppressWarnings("unchecked")
    void canonicalizesMapAndSetIterationWithoutCallingUserCode() {
        Map<String, Integer> firstMap = new HashMap<>();
        firstMap.put("FB", 4);
        firstMap.put("Ea", 0);
        Map<String, Integer> secondMap = new HashMap<>();
        secondMap.put("Ea", 0);
        secondMap.put("FB", 4);

        Map<String, Object> firstCaptured = (Map<String, Object>) SafeValue.capture(firstMap);
        Map<String, Object> secondCaptured = (Map<String, Object>) SafeValue.capture(secondMap);
        assertEquals(firstCaptured.get("entries"), secondCaptured.get("entries"));

        var firstSet = new HashSet<>(List.of("FB", "Ea"));
        var secondSet = new HashSet<>(List.of("Ea", "FB"));
        Map<String, Object> firstSetCapture = (Map<String, Object>) SafeValue.capture(firstSet);
        Map<String, Object> secondSetCapture = (Map<String, Object>) SafeValue.capture(secondSet);
        assertEquals(firstSetCapture.get("items"), secondSetCapture.get("items"));

        Map<String, Integer> oversizedUnordered = new HashMap<>();
        for (int index = 0; index < 100; index++) oversizedUnordered.put("key-" + index, index);
        Map<String, Object> oversizedCapture =
                (Map<String, Object>) SafeValue.capture(oversizedUnordered);
        assertEquals(List.of(), oversizedCapture.get("entries"));
        assertEquals(true, oversizedCapture.get("truncated"));
    }

    @Test
    void immutableMapCaptureIsStableAcrossFreshJvmSalts() throws Exception {
        String javaExecutable = Path.of(System.getProperty("java.home"), "bin", "java").toString();
        String classPath = String.join(
                java.io.File.pathSeparator,
                Path.of(SafeValueTest.class.getProtectionDomain().getCodeSource().getLocation().toURI())
                        .toString(),
                Path.of(SafeValue.class.getProtectionDomain().getCodeSource().getLocation().toURI())
                        .toString());
        var representations = new HashSet<String>();
        for (int attempt = 0; attempt < 12; attempt++) {
            Process process = new ProcessBuilder(
                            javaExecutable,
                            "-cp",
                            classPath,
                            SafeValueProcessProbe.class.getName())
                    .start();
            assertTrue(process.waitFor(5, TimeUnit.SECONDS));
            assertEquals(0, process.exitValue());
            representations.add(new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8));
        }

        assertEquals(1, representations.size(), representations.toString());
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
