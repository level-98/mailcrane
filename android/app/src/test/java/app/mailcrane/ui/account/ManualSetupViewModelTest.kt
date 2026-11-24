package app.mailcrane.ui.account

import app.mailcrane.core.AddAccountException
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.Field
import app.mailcrane.core.FieldError
import app.mailcrane.core.FieldProblem
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ManualSetupViewModelTest {
    private class FakeCore(
        private val result: () -> Long,
    ) : MailcraneCore {
        var lastConfig: ManualAccountConfig? = null

        override fun hello() = ""

        override fun defaultPort(
            protocol: MailServer,
            security: ConnectionSecurity,
        ) = when (protocol to security) {
            MailServer.IMAP to ConnectionSecurity.TLS -> 993
            MailServer.IMAP to ConnectionSecurity.START_TLS -> 143
            MailServer.SMTP to ConnectionSecurity.TLS -> 465
            else -> 587
        }

        override suspend fun addAccountManual(
            config: ManualAccountConfig,
            password: String,
        ): Long {
            lastConfig = config
            return result()
        }
    }

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun startsWithDefaultPortsAndSecurityChangeFillsPort() {
        val vm = ManualSetupViewModel(FakeCore { 1 })
        assertEquals("993", vm.state.value.imapPort)
        assertEquals("465", vm.state.value.smtpPort)
        vm.onImapSecurityChange(ConnectionSecurity.START_TLS)
        vm.onSmtpSecurityChange(ConnectionSecurity.START_TLS)
        assertEquals("143", vm.state.value.imapPort)
        assertEquals("587", vm.state.value.smtpPort)
    }

    @Test
    fun successReportsAccountIdAndSendsTrimmedConfig() {
        val core = FakeCore { 42 }
        val vm = ManualSetupViewModel(core)
        vm.onEmailChange(" alice@example.org ")
        vm.onImapHostChange("imap.example.org")
        vm.onImapPortChange("99x3")
        vm.submit()
        assertEquals(42L, vm.state.value.addedAccountId)
        assertEquals("alice@example.org", core.lastConfig?.email)
        assertEquals(993u, core.lastConfig?.imap?.port)
    }

    @Test
    fun fieldErrorsAreShownAndClearedOnEdit() {
        val vm =
            ManualSetupViewModel(
                FakeCore {
                    throw AddAccountException.Invalid(listOf(FieldError(Field.EMAIL, FieldProblem.MALFORMED_EMAIL)))
                },
            )
        vm.onEmailChange("nope")
        vm.submit()
        assertEquals(FieldProblem.MALFORMED_EMAIL, vm.state.value.fieldErrors[Field.EMAIL])
        assertEquals("nope", vm.state.value.email)
        vm.onEmailChange("nope@example.org")
        assertNull(vm.state.value.fieldErrors[Field.EMAIL])
    }

    @Test
    fun serverErrorsMapToMessagesAndKeepInput() {
        val cases =
            listOf(
                AddAccountException.CannotConnect(MailServer.IMAP) to
                    SetupError(SetupFailure.CANNOT_CONNECT, MailServer.IMAP),
                AddAccountException.TlsFailure(
                    MailServer.SMTP,
                ) to SetupError(SetupFailure.TLS_FAILURE, MailServer.SMTP),
                AddAccountException.StartTlsNotOffered(MailServer.IMAP) to
                    SetupError(SetupFailure.STARTTLS_NOT_OFFERED, MailServer.IMAP),
                AddAccountException.AuthFailed(
                    MailServer.SMTP,
                ) to SetupError(SetupFailure.AUTH_FAILED, MailServer.SMTP),
                AddAccountException.Timeout(MailServer.IMAP) to SetupError(SetupFailure.TIMEOUT, MailServer.IMAP),
                AddAccountException.SaveFailed() to SetupError(SetupFailure.SAVE_FAILED),
                IllegalStateException("boom") to SetupError(SetupFailure.UNEXPECTED),
            )
        for ((thrown, expected) in cases) {
            val vm = ManualSetupViewModel(FakeCore { throw thrown })
            vm.onPasswordChange("pw")
            vm.submit()
            assertEquals(expected, vm.state.value.error)
            assertFalse(vm.state.value.checking)
            assertEquals("pw", vm.state.value.password)
        }
    }
}
