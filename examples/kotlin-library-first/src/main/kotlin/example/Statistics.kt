package example

import io.github.p4suta.walaru.Walaru

fun median(values: List<Int>, accessToken: String): Int = Walaru.span("median").use { span ->
    span.capture("inputSize", values.size)
    val ordered = values.sorted()
    val middle = Walaru.capture("middleIndex", ordered.size / 2)
    Walaru.checkpoint("ordered", ordered)
    Walaru.captureRedacted("accessToken", accessToken)
    ordered[middle - 1] // Intentional bug for odd-sized input.
}
