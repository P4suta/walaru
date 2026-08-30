package demo

import io.github.p4suta.walaru.Walaru
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
    return Walaru.span("inspect snapshot").use { span ->
        span.capture("seed", seed)
        delay(1)
        val value = Walaru.capture("counterValue", Counter(seed).increment())
        Walaru.checkpoint("snapshot", mapOf("seed" to seed, "value" to value))
        Walaru.captureRedacted("apiToken", "fixture-secret")
        Snapshot(value)
    }
}
