package io.github.p4suta.walaru.model;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

final class KotlinSourceMapTest {
    @Test
    void exposesStableInlineSourcePositionsToExternalJvmConsumers() {
        KotlinSourceMap map = KotlinSourceMap.parse(
                "SMAP\nCaller.kt\nKotlin\n*S Kotlin\n*F\n"
                        + "+ 1 Caller.kt\ndemo/Caller.kt\n"
                        + "+ 2 Shared.kt\ndemo/shared/Shared.kt\n"
                        + "*L\n8#1:100\n3#2,2:101\n*E\n",
                "Caller.kt");

        assertEquals(new KotlinSourceMap.Position("demo/Caller.kt", 8), map.position(100));
        assertEquals(new KotlinSourceMap.Position("demo/shared/Shared.kt", 3), map.position(101));
        assertEquals(new KotlinSourceMap.Position("demo/shared/Shared.kt", 4), map.position(102));
    }
}
