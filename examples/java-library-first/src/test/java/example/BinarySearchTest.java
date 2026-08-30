package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

final class BinarySearchTest {
    @Test
    void findsTheLastValue() {
        assertEquals(4, BinarySearch.find(new int[] {1, 3, 5, 7, 9}, 9, "example-secret"));
    }
}
