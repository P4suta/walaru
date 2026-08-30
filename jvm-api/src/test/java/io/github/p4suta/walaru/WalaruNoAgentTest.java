package io.github.p4suta.walaru;

import static org.junit.jupiter.api.Assertions.assertDoesNotThrow;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.jupiter.api.Test;

class WalaruNoAgentTest {
    @Test
    void every_operation_is_a_noop_without_the_agent() {
        Object value = new Object();

        assertFalse(Walaru.active());
        assertFalse(Walaru.context().present());
        assertSame(value, Walaru.capture("value", value));
        assertSame(value, Walaru.captureRedacted("secret", value));
        assertSame(value, Walaru.checkpoint("point", value));
        AtomicBoolean evaluated = new AtomicBoolean();
        Walaru.captureLazy("expensive", () -> {
            evaluated.set(true);
            return value;
        });
        assertFalse(evaluated.get());
        assertDoesNotThrow(() -> {
            Walaru.checkpoint(null);
            Walaru.note("phase", "ready");
            Walaru.note("ready");
            Walaru.captureLazy("ignored", null);
            Walaru.context().wrap(() -> {}).run();
            assertEquals("ok", Walaru.context().wrapSupplier(() -> "ok").get());
            try (WalaruSpan span = Walaru.span("work")) {
                span.capture("value", value).captureRedacted("token", value);
            }
        });
    }

    @Test
    void span_helpers_preserve_results_and_failures() throws Exception {
        assertSame("ok", Walaru.call("call", () -> "ok"));
        IllegalStateException failure = assertThrows(
                IllegalStateException.class,
                () -> Walaru.run("run", () -> { throw new IllegalStateException("boom"); }));
        assertEquals("boom", failure.getMessage());
    }
}
