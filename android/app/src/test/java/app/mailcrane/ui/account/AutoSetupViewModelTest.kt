package app.mailcrane.ui.account

import app.mailcrane.core.AddAccountException
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.DiscoverException
import app.mailcrane.core.DiscoveredSettings
import app.mailcrane.core.Field
import app.mailcrane.core.FieldProblem
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import app.mailcrane.core.ServerConfig
import app.mailcrane.core.SettingsSource
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

/** Automatic setup (M1-15): email first, then confirm or fall through to the form. */
@OptIn(ExperimentalCoroutinesApi::class)
class AutoSetupViewModelTest {
    private class FakeCore(
        private val discovered: () -> List<DiscoveredSettings>,
        private val added: () -> Long = { 7 },
    ) : MailcraneCore {
        val looked = mutableListOf<String>()
        var lastConfig: ManualAccountConfig? = null
        var lastPassword: String? = null

        override fun hello() = ""

        override fun defaultPort(
            protocol: MailServer,
            security: ConnectionSecurity,
        ) = if (protocol == MailServer.IMAP) 993 else 465

        override suspend fun discoverSettings(email: String): List<DiscoveredSettings> {
            looked += email
            return discovered()
        }

        override suspend fun addAccountManual(
            config: ManualAccountConfig,
            password: String,
        ): Long {
            lastConfig = config
            lastPassword = password
            return added()
        }
    }

    private val found =
        DiscoveredSettings(
            source = SettingsSource.AUTOCONFIG,
            sourceDomain = "example.org",
            providerName = "Example Mail",
            imap = ServerConfig("imap.example.org", 993u, ConnectionSecurity.TLS),
            smtp = ServerConfig("smtp.example.org", 587u, ConnectionSecurity.START_TLS),
            username = "ana@example.org",
        )

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    private fun ManualSetupViewModel.typeCredentials() {
        onEmailChange(" ana@example.org ")
        onPasswordChange("app-password")
    }

    @Test
    fun startsOnTheEmailStep() {
        val vm = ManualSetupViewModel(FakeCore({ emptyList() }))
        assertEquals(SetupStep.EMAIL, vm.state.value.step)
    }

    @Test
    fun foundSettingsFillTheFormAndShowForConfirmation() {
        val core = FakeCore({ listOf(found) })
        val vm = ManualSetupViewModel(core)
        vm.typeCredentials()
        vm.discover()
        val s = vm.state.value
        assertEquals(listOf("ana@example.org"), core.looked)
        assertEquals(SetupStep.CONFIRM, s.step)
        assertEquals(found, s.found)
        assertEquals("smtp.example.org", s.smtpHost)
        assertEquals("587", s.smtpPort)
        assertEquals(ConnectionSecurity.START_TLS, s.smtpSecurity)
        assertFalse(s.discovering)

        vm.submit()
        assertEquals(7L, vm.state.value.addedAccountId)
        assertEquals("imap.example.org", core.lastConfig?.imap?.host)
        assertEquals("ana@example.org", core.lastConfig?.username)
        assertEquals("app-password", core.lastPassword)
    }

    @Test
    fun editSettingsOpensTheFilledForm() {
        val vm = ManualSetupViewModel(FakeCore({ listOf(found) }))
        vm.typeCredentials()
        vm.discover()
        vm.editSettings()
        assertEquals(SetupStep.MANUAL, vm.state.value.step)
        assertEquals("imap.example.org", vm.state.value.imapHost)
    }

    @Test
    fun nothingFoundFallsThroughToTheFormWithANote() {
        val vm = ManualSetupViewModel(FakeCore({ throw DiscoverException.NotFound() }))
        vm.typeCredentials()
        vm.discover()
        val s = vm.state.value
        assertEquals(SetupStep.MANUAL, s.step)
        assertEquals(DiscoveryNote.NOT_FOUND, s.discoveryNote)
        assertEquals("ana@example.org", s.username)
        assertEquals("app-password", s.password)
    }

    @Test
    fun aTimeoutAndUnexpectedErrorsAlsoFallThrough() {
        val slow = ManualSetupViewModel(FakeCore({ throw DiscoverException.Timeout() }))
        slow.typeCredentials()
        slow.discover()
        assertEquals(DiscoveryNote.TIMED_OUT, slow.state.value.discoveryNote)
        val broken = ManualSetupViewModel(FakeCore({ throw IllegalStateException("boom") }))
        broken.typeCredentials()
        broken.discover()
        assertEquals(SetupStep.MANUAL, broken.state.value.step)
        assertEquals(DiscoveryNote.NOT_FOUND, broken.state.value.discoveryNote)
    }

    @Test
    fun missingAddressOrPasswordIsCaughtBeforeLookingUp() {
        val core = FakeCore({ listOf(found) })
        val vm = ManualSetupViewModel(core)
        vm.onEmailChange("not an address")
        vm.discover()
        assertEquals(FieldProblem.MALFORMED_EMAIL, vm.state.value.fieldErrors[Field.EMAIL])
        assertEquals(FieldProblem.EMPTY, vm.state.value.fieldErrors[Field.PASSWORD])
        assertEquals(SetupStep.EMAIL, vm.state.value.step)
        assertEquals(emptyList<String>(), core.looked)
    }

    @Test
    fun setUpManuallySkipsTheLookup() {
        val core = FakeCore({ listOf(found) })
        val vm = ManualSetupViewModel(core)
        vm.onEmailChange("ana@example.org")
        vm.setUpManually()
        assertEquals(SetupStep.MANUAL, vm.state.value.step)
        assertEquals("ana@example.org", vm.state.value.username)
        assertNull(vm.state.value.discoveryNote)
        assertEquals(emptyList<String>(), core.looked)
    }

    @Test
    fun aLoginFailureStaysOnConfirmWithTheError() {
        val vm =
            ManualSetupViewModel(
                FakeCore({ listOf(found) }, { throw AddAccountException.AuthFailed(MailServer.IMAP) }),
            )
        vm.typeCredentials()
        vm.discover()
        vm.submit()
        assertEquals(SetupStep.CONFIRM, vm.state.value.step)
        assertEquals(SetupError(SetupFailure.AUTH_FAILED, MailServer.IMAP), vm.state.value.error)
    }
}
