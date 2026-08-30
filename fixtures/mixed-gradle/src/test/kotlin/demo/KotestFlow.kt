package demo

import io.kotest.core.spec.style.FunSpec
import io.kotest.matchers.shouldBe

class KotestFlow : FunSpec({
    test("suspend path") {
        inspect(2).value shouldBe 3
    }
})
