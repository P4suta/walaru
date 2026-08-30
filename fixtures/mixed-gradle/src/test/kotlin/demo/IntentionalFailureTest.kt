package demo

import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class IntentionalFailureTest {
    @Test
    fun explainsFailure() = runBlocking {
        assertEquals(99, inspect(1).value, "expected token=fixture-secret")
    }
}
