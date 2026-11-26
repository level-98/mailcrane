package app.mailcrane.ui.account

import app.mailcrane.core.AddAccountException
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.Field
import app.mailcrane.core.FieldError
import app.mailcrane.core.FieldProblem
import app.mailcrane.core.MailServer
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
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun startsWithDefaultPortsAndSecurityChangeFillsPort() {
        val vm = ManualSetupViewModel(FakeSetupCore { 1 })
        assertEquals("993", vm.state.value.imapPort)
        assertEquals("465", vm.state.value.smtpPort)
        vm.onImapSecurityChange(ConnectionSecurity.START_TLS)
        vm.onSmtpSecurityChange(ConnectionSecurity.START_TLS)
        assertEquals("143", vm.state.value.imapPort)
        assertEquals("587", vm.state.value.smtpPort)
    }

    @Test
    fun successReportsAccountIdAndSendsTrimmedConfig() {
        val core = FakeSetupCore { 42 }
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
                FakeSetupCore {
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
                AddAccountException.AlreadyAdded() to SetupError(SetupFailure.ALREADY_ADDED),
                IllegalStateException("boom") to SetupError(SetupFailure.UNEXPECTED),
            )
        for ((thrown, expected) in cases) {
            val vm = ManualSetupViewModel(FakeSetupCore { throw thrown })
            vm.onPasswordChange("pw")
            vm.submit()
            assertEquals(expected, vm.state.value.error)
            assertFalse(vm.state.value.checking)
            assertEquals("pw", vm.state.value.password)
        }
    }

    @Test
    fun jmapSetupSendsTheServerInsteadOfImapAndSmtp() {
        val core = FakeSetupCore { 7 }
        val vm = ManualSetupViewModel(core)
        vm.setUpManually()
        vm.onJmapChange(true)
        vm.onEmailChange("ana@example.org")
        vm.onJmapServerChange(" example.org ")
        vm.onUsernameChange("ana@example.org")
        vm.onPasswordChange("app-password")
        vm.submit()
        assertEquals(7L, vm.state.value.addedAccountId)
        assertEquals("example.org", core.lastJmap?.server)
        assertEquals("ana@example.org", core.lastJmap?.username)
        assertNull("no IMAP setup was attempted", core.lastConfig)
    }

    @Test
    fun aBadJmapServerIsShownOnItsField() {
        val vm =
            ManualSetupViewModel(
                FakeSetupCore {
                    throw AddAccountException.Invalid(listOf(FieldError(Field.IMAP_HOST, FieldProblem.EMPTY)))
                },
            )
        vm.onJmapChange(true)
        vm.submit()
        assertEquals(FieldProblem.EMPTY, vm.state.value.fieldErrors[Field.IMAP_HOST])
        vm.onJmapServerChange("example.org")
        assertNull(vm.state.value.fieldErrors[Field.IMAP_HOST])
    }

    @Test
    fun handlingTheNewAccountEmptiesTheForm() {
        // #343: the activity keeps this view model, so the next "Add account" must start empty.
        val vm = ManualSetupViewModel(FakeSetupCore { 42 })
        vm.onEmailChange("alice@example.org")
        vm.onPasswordChange("secret")
        vm.submit()
        assertEquals(42L, vm.state.value.addedAccountId)
        vm.accountHandled()
        assertNull(vm.state.value.addedAccountId)
        assertEquals("", vm.state.value.email)
        assertEquals("", vm.state.value.password)
        assertEquals(SetupStep.EMAIL, vm.state.value.step)
        assertEquals("993", vm.state.value.imapPort)
    }

    @Test
    fun aDuplicateAccountIsSaidPlainly() {
        // #346: the core refuses a second copy of an account; the screen says why.
        assertEquals(
            app.mailcrane.R.string.setup_error_already_added,
            SetupError(SetupFailure.ALREADY_ADDED).message(),
        )
    }
}
