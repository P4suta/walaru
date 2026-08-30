package io.github.p4suta.walaru.agent;

import java.lang.reflect.Array;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

/** Bounded field capture that never invokes user getters or {@code toString()}. */
final class SafeValue {
    private static final int MAX_DEPTH = 3;
    private static final int MAX_FIELDS = 24;
    private static final int MAX_ARRAY = 32;
    private static final int MAX_STRING = 256;
    private static final Pattern SECRET_VALUE = Pattern.compile(
            "(?i)(?:^|[\\s_:=\\-])(?:password|passwd|secret|token|credential|api[_-]?key|private[_-]?key)(?:$|[\\s_:=\\-])"
                    + "|^Bearer\\s+\\S+$"
                    + "|^AKIA[0-9A-Z]{16}$"
                    + "|^[A-Za-z0-9_-]{16,}\\.[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}$");

    private SafeValue() {}

    static Object capture(Object value) {
        return capture(value, 0, new IdentityHashMap<>());
    }

    static Object captureNamed(String name, Object value) {
        return secretName(name) ? "<redacted>" : capture(value);
    }

    private static Object capture(Object value, int depth, IdentityHashMap<Object, Boolean> seen) {
        if (value == null || value instanceof Boolean || trustedNumber(value)) return value;
        if (value instanceof Character character) return character.toString();
        if (value instanceof String string) {
            return secretValue(string) ? "<redacted>" : truncate(string);
        }
        if (value instanceof Enum<?> enumeration) return enumeration.name();
        if (seen.put(value, Boolean.TRUE) != null) {
            return Map.of("type", value.getClass().getName(), "cycle", true);
        }
        try {
            if (depth >= MAX_DEPTH) {
                return Map.of("type", value.getClass().getName(), "truncated", true);
            }
            if (value.getClass().isArray()) {
                int length = Array.getLength(value);
                List<Object> items = new ArrayList<>(Math.min(length, MAX_ARRAY));
                for (int index = 0; index < Math.min(length, MAX_ARRAY); index++) {
                    items.add(capture(Array.get(value, index), depth + 1, seen));
                }
                Map<String, Object> array = new LinkedHashMap<>();
                array.put("type", value.getClass().getTypeName());
                array.put("length", length);
                array.put("items", items);
                array.put("truncated", length > MAX_ARRAY);
                return array;
            }
            if (value instanceof Collection<?> collection && trustedContainer(value.getClass())) {
                int length = collection.size();
                List<Object> items = new ArrayList<>(Math.min(length, MAX_ARRAY));
                boolean unordered = value instanceof Set<?> && unorderedSet(value.getClass());
                if (!unordered || length <= MAX_ARRAY) {
                    int index = 0;
                    for (Object item : collection) {
                        if (index++ >= MAX_ARRAY) break;
                        items.add(capture(item, depth + 1, seen));
                    }
                }
                if (unordered) {
                    items.sort(Comparator.comparing(SafeValue::stableSortKey));
                }
                Map<String, Object> captured = new LinkedHashMap<>();
                captured.put("type", value.getClass().getName());
                captured.put("length", length);
                captured.put("items", items);
                captured.put("truncated", length > MAX_ARRAY);
                return captured;
            }
            if (value instanceof Map<?, ?> map && trustedContainer(value.getClass())) {
                int length = map.size();
                List<Map<String, Object>> entries = new ArrayList<>(Math.min(length, MAX_ARRAY));
                boolean unordered = unorderedMap(value.getClass());
                if (!unordered || length <= MAX_ARRAY) {
                    int index = 0;
                    for (Map.Entry<?, ?> entry : map.entrySet()) {
                        if (index++ >= MAX_ARRAY) break;
                        Map<String, Object> capturedEntry = new LinkedHashMap<>();
                        capturedEntry.put("key", capture(entry.getKey(), depth + 1, seen));
                        capturedEntry.put(
                                "value",
                                entry.getKey() instanceof String name && secretName(name)
                                        ? "<redacted>"
                                        : capture(entry.getValue(), depth + 1, seen));
                        entries.add(capturedEntry);
                    }
                }
                if (unordered) {
                    entries.sort(Comparator
                            .comparing((Map<String, Object> entry) -> stableSortKey(entry.get("key")))
                            .thenComparing(entry -> stableSortKey(entry.get("value"))));
                }
                Map<String, Object> captured = new LinkedHashMap<>();
                captured.put("type", value.getClass().getName());
                captured.put("length", length);
                captured.put("entries", entries);
                captured.put("truncated", length > MAX_ARRAY);
                return captured;
            }

            Map<String, Object> fields = new LinkedHashMap<>();
            Class<?> current = value.getClass();
            while (current != null && current != Object.class && fields.size() < MAX_FIELDS) {
                for (Field field : current.getDeclaredFields()) {
                    if (fields.size() >= MAX_FIELDS) break;
                    if (Modifier.isStatic(field.getModifiers()) || field.isSynthetic()) continue;
                    if (secretName(field.getName())) {
                        fields.put(field.getName(), "<redacted>");
                        continue;
                    }
                    try {
                        if (field.trySetAccessible()) {
                            fields.put(field.getName(), capture(field.get(value), depth + 1, seen));
                        } else {
                            fields.put(field.getName(), "<inaccessible>");
                        }
                    } catch (IllegalAccessException | RuntimeException inaccessible) {
                        fields.put(field.getName(), "<inaccessible>");
                    }
                }
                current = current.getSuperclass();
            }
            Map<String, Object> object = new LinkedHashMap<>();
            object.put("type", value.getClass().getName());
            object.put("fields", fields);
            object.put("truncated", fields.size() >= MAX_FIELDS);
            return object;
        } finally {
            seen.remove(value);
        }
    }

    private static boolean secretName(String name) {
        String lower = name.toLowerCase(Locale.ROOT);
        return lower.contains("password")
                || lower.contains("secret")
                || lower.contains("token")
                || lower.contains("credential")
                || lower.equals("key")
                || lower.endsWith("key");
    }

    private static boolean trustedNumber(Object value) {
        Class<?> type = value.getClass();
        return type == Byte.class
                || type == Short.class
                || type == Integer.class
                || type == Long.class
                || type == Float.class
                || type == Double.class
                || type == BigInteger.class
                || type == BigDecimal.class;
    }

    private static boolean trustedContainer(Class<?> type) {
        if (type.getClassLoader() != null) return false;
        String name = type.getName();
        return name.equals("java.util.ArrayList")
                || name.equals("java.util.LinkedList")
                || name.equals("java.util.ArrayDeque")
                || name.equals("java.util.HashSet")
                || name.equals("java.util.LinkedHashSet")
                || name.equals("java.util.TreeSet")
                || name.equals("java.util.HashMap")
                || name.equals("java.util.LinkedHashMap")
                || name.equals("java.util.TreeMap")
                || name.equals("java.util.EnumMap")
                || name.startsWith("java.util.ImmutableCollections$");
    }

    private static boolean unorderedMap(Class<?> type) {
        String name = type.getName();
        return name.equals("java.util.HashMap")
                || name.startsWith("java.util.ImmutableCollections$Map");
    }

    private static boolean unorderedSet(Class<?> type) {
        String name = type.getName();
        return name.equals("java.util.HashSet")
                || name.startsWith("java.util.ImmutableCollections$Set");
    }

    private static boolean secretValue(String value) {
        return SECRET_VALUE.matcher(value).find();
    }

    private static String stableSortKey(Object value) {
        StringBuilder key = new StringBuilder();
        appendStable(key, value);
        return key.toString();
    }

    private static void appendStable(StringBuilder output, Object value) {
        if (value == null) {
            output.append('0');
        } else if (value instanceof CharSequence text) {
            output.append('s').append(text.length()).append(':').append(text);
        } else if (value instanceof Number number) {
            output.append('n').append(number.getClass().getName()).append(':').append(number);
        } else if (value instanceof Boolean bool) {
            output.append('b').append(bool);
        } else if (value instanceof Map<?, ?> map) {
            output.append('{');
            map.entrySet().stream()
                    .sorted(Comparator.comparing(entry -> stableSortKey(entry.getKey())))
                    .forEach(entry -> {
                        appendStable(output, entry.getKey());
                        appendStable(output, entry.getValue());
                    });
            output.append('}');
        } else if (value instanceof Iterable<?> iterable) {
            output.append('[');
            for (Object item : iterable) appendStable(output, item);
            output.append(']');
        } else {
            output.append('c').append(value.getClass().getName());
        }
    }

    private static String truncate(String value) {
        return value.length() <= MAX_STRING ? value : value.substring(0, MAX_STRING) + "…";
    }
}
