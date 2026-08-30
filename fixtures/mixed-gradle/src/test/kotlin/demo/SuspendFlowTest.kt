package demo

import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class SuspendFlowTest {
    @Test
    fun recordsValues() = runBlocking {
        assertEquals(4, JavaBoundary.twice(inspect(1).value))
    }
}
