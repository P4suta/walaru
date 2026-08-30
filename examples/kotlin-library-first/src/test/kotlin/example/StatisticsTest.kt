package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class StatisticsTest {
    @Test
    fun `odd-sized input returns its middle value`() {
        assertEquals(5, median(listOf(9, 1, 5), "example-secret"))
    }
}
