package app.mailcrane.ui

import app.mailcrane.core.MailcraneCore
import org.junit.Assert.assertEquals
import org.junit.Test

class HelloViewModelTest {
    private class FakeCore(
        private val result: () -> String,
    ) : MailcraneCore {
        override fun hello(): String = result()
    }

    @Test
    fun exposesGreetingFromCore() {
        val viewModel = HelloViewModel(FakeCore { "Hello from fake core" })
        assertEquals("Hello from fake core", viewModel.greeting)
    }

    @Test
    fun failingCoreDoesNotCrash() {
        val viewModel = HelloViewModel(FakeCore { error("boom") })
        assertEquals("", viewModel.greeting)
    }
}
