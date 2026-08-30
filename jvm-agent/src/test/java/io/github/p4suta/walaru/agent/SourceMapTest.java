package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

final class SourceMapTest {
    @Test
    void mapsKotlinSmapOutputLinesAndFallsBackForUnknownForms() {
        SourceMap map = SourceMap.parse(
                "SMAP\nExample.kt\nKotlin\n*S Kotlin\n*F\n+ 1 Example.kt\ndemo/Example.kt\n*L\n7#1,3:20\n*E\n",
                "Example.kt");

        assertEquals(7, map.map(20));
        assertEquals(9, map.map(22));
        assertEquals(100, map.map(100));
        assertEquals("demo/Example.kt", map.sourcePath());
    }

    @Test
    void mapsEachInlineRangeToItsOwnKotlinSourceFile() {
        SourceMap map = SourceMap.parse(
                "SMAP\nCaller.kt\nKotlin\n*S Kotlin\n*F\n"
                        + "+ 1 Caller.kt\ndemo/Caller.kt\n"
                        + "+ 2 Library.kt\ndemo/library/Library.kt\n"
                        + "*L\n10#1:100\n4#2,2:101\n*E\n",
                "Caller.kt");

        assertEquals(new SourceMap.Position("demo/Caller.kt", 10), map.position(100));
        assertEquals(new SourceMap.Position("demo/library/Library.kt", 4), map.position(101));
        assertEquals(new SourceMap.Position("demo/library/Library.kt", 5), map.position(102));
    }
}
