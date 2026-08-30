package io.github.p4suta.walaru.agent;

import java.io.IOException;
import java.io.StringWriter;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.Map;

final class EventWriter implements AutoCloseable {
    private final FileChannel channel;

    EventWriter(Path path) throws IOException {
        Path parent = path.toAbsolutePath().normalize().getParent();
        if (parent != null) Files.createDirectories(parent);
        channel = FileChannel.open(
                path,
                StandardOpenOption.CREATE,
                StandardOpenOption.APPEND,
                StandardOpenOption.WRITE);
    }

    synchronized void write(Map<String, ?> fields) {
        try {
            StringWriter writer = new StringWriter();
            writeValue(writer, fields);
            writer.write('\n');
            ByteBuffer bytes = StandardCharsets.UTF_8.encode(writer.toString());
            try (var lock = channel.lock()) {
                if (!lock.isValid()) throw new IOException("event stream lock became invalid");
                channel.position(channel.size());
                while (bytes.hasRemaining()) channel.write(bytes);
                channel.force(false);
            }
        } catch (IOException failure) {
            throw new IllegalStateException("cannot write Walaru event stream", failure);
        }
    }

    private static void writeValue(StringWriter writer, Object value) {
        if (value == null) {
            writer.write("null");
        } else if (value instanceof Number || value instanceof Boolean) {
            writer.write(String.valueOf(value));
        } else if (value instanceof CharSequence text) {
            writeString(writer, text.toString());
        } else if (value instanceof Map<?, ?> map) {
            writer.write('{');
            boolean first = true;
            for (Map.Entry<?, ?> field : map.entrySet()) {
                if (!first) writer.write(',');
                first = false;
                writeString(writer, (String) field.getKey());
                writer.write(':');
                writeValue(writer, field.getValue());
            }
            writer.write('}');
        } else if (value instanceof Iterable<?> iterable) {
            writer.write('[');
            boolean first = true;
            for (Object item : iterable) {
                if (!first) writer.write(',');
                first = false;
                writeValue(writer, item);
            }
            writer.write(']');
        } else {
            writeString(writer, value.getClass().getName());
        }
    }

    private static void writeString(StringWriter writer, String value) {
        writer.write('"');
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            switch (character) {
                case '"' -> writer.write("\\\"");
                case '\\' -> writer.write("\\\\");
                case '\n' -> writer.write("\\n");
                case '\r' -> writer.write("\\r");
                case '\t' -> writer.write("\\t");
                default -> {
                    if (character < 0x20) writer.write(String.format("\\u%04x", (int) character));
                    else writer.write(character);
                }
            }
        }
        writer.write('"');
    }

    @Override
    public synchronized void close() throws IOException {
        channel.close();
    }
}
