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
    fun gmailSignsInWithGoogleAndAddsTheAccount() {
        // M3-02: a Gmail address goes through Sign in with Google.
        val core = FakeSetupCore({ 9 })
        val vm = ManualSetupViewModel(core, googleSignIn = true)
        vm.onEmailChange("ann@gmail.com")
        assertEquals(true, vm.state.value.gmail)
        val (provider, email) = vm.startGoogleSignIn() ?: error("started")
        assertEquals("google", provider.id)
        assertEquals("ann@gmail.com", email)
        assertEquals(true, vm.state.value.checking)

        vm.onGoogleResult(
            app.mailcrane.oauth.OAuthResult
                .SignedIn("signin-1"),
        )
        assertEquals(9L, vm.state.value.addedAccountId)
        assertEquals(Triple("signin-1", "ann@gmail.com", ""), core.lastGoogle)
    }

    @Test
    fun googleSignInNeedsAnAddressAndReportsFailures() {
        val vm = ManualSetupViewModel(FakeSetupCore({ 9 }), googleSignIn = true)
        assertNull(vm.startGoogleSignIn())
        assertEquals(FieldProblem.MALFORMED_EMAIL, vm.state.value.fieldErrors[Field.EMAIL])

        vm.onEmailChange("ann@example.com")
        assertEquals(false, vm.state.value.gmail)
        vm.startGoogleSignIn()
        // Cancelled in the browser: back to the form, no error.
        vm.onGoogleResult(
            app.mailcrane.oauth.OAuthResult
                .Failed(
                    app.mailcrane.core.OAuthFailure
                        .Denied(),
                ),
        )
        assertFalse(vm.state.value.checking)
        assertNull(vm.state.value.error)
        // Anything else (or no browser) says so.
        vm.startGoogleSignIn()
        vm.onGoogleResult(
            app.mailcrane.oauth.OAuthResult
                .Failed(null),
        )
        assertEquals(SetupError(SetupFailure.GOOGLE_SIGN_IN_FAILED), vm.state.value.error)
        assertEquals(
            app.mailcrane.R.string.setup_error_google,
            SetupError(SetupFailure.GOOGLE_SIGN_IN_FAILED).message(),
        )
    }

    @Test
    fun aDuplicateAccountIsSaidPlainly() {
        // #346: the core refuses a second copy of an account; the screen says why.
        assertEquals(
            app.mailcrane.R.string.setup_error_already_added,
            SetupError(SetupFailure.ALREADY_ADDED).message(),
        )
    }

    @Test
    fun closingTheBrowserTabLeavesSigningInButAReturnStaysWaiting() {
        // #376: back in the app with nothing from the browser means the tab was closed.
        val vm = ManualSetupViewModel(FakeSetupCore({ 9 }), googleSignIn = true)
        vm.onEmailChange("ann@gmail.com")
        vm.startGoogleSignIn()
        assertEquals(true, vm.state.value.waitingForGoogle)
        vm.googleNotReturned()
        assertFalse(vm.state.value.checking)
        assertNull(vm.state.value.error)

        // A redirect that arrived: the code is being exchanged, so don't give up.
        vm.startGoogleSignIn()
        vm.onGoogleResult(app.mailcrane.oauth.OAuthResult.Returned)
        vm.googleNotReturned()
        assertEquals(true, vm.state.value.checking)
    }

    @Test
    fun withoutGoogleSignInAGmailAddressUsesAPassword() {
        // #405: debug builds can't receive Google's redirect, so they don't offer it. A Gmail
        // address takes the password path (an app password) and no sign-in starts.
        val core = FakeSetupCore({ 9 })
        val vm = ManualSetupViewModel(core, googleSignIn = false)
        vm.onEmailChange("ann@gmail.com")
        assertEquals(false, vm.state.value.gmail)
        assertNull(vm.startGoogleSignIn())
        assertFalse(vm.state.value.checking)
        assertFalse(vm.state.value.waitingForGoogle)
        assertNull(core.lastGoogle)
    }

    @Test
    fun onlyReleaseBuildsOfferGoogleSignIn() {
        // Unit tests run against the debug build.
        assertFalse(app.mailcrane.oauth.OAuthSignIn.AVAILABLE)
        assertEquals("app.mailcrane:/oauth2redirect", app.mailcrane.oauth.OAuthSignIn.REDIRECT_URI)
    }
}
