package demo

import kotlinx.coroutines.delay

data class Snapshot(
    val value: Int,
    val apiToken: String = "fixture-secret",
)

class Counter(var value: Int) {
    fun increment(): Int {
        value += 1
        return value
    }
}

suspend fun inspect(seed: Int): Snapshot {
    delay(1)
    return Snapshot(Counter(seed).increment())
}
