package app.mailcrane.ui

import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import org.junit.Assert.assertEquals
import org.junit.Test

class HelloViewModelTest {
    private class FakeCore(
        private val result: () -> String,
    ) : MailcraneCore {
        override fun hello(): String = result()

        override fun defaultPort(
            protocol: MailServer,
            security: ConnectionSecurity,
        ): Int = 0

        override suspend fun addAccountManual(
            config: ManualAccountConfig,
            password: String,
        ): Long = error("not used")
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
