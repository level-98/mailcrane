package app.mailcrane.settings

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import app.mailcrane.core.AccountSettingsCore
import app.mailcrane.core.SenderTrustChanges
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test

/**
 * #492 review: closing an account's page clears its view models (#500). A save already under way must
 * still finish its follow-up: the trust refresh after an authentication server name, the account list
 * refresh after a colour.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SaveSurvivesCloseTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    /** A core whose writes wait for [gate], so the page can close while one is under way. */
    private class GatedSettings(
        private val delegate: FakeSettings = FakeSettings(),
    ) : AccountSettingsCore by delegate {
        val gate = CompletableDeferred<Unit>()
        var authserv: String? = null
        var color: Int? = null

        override suspend fun setProviderAuthservId(
            accountId: Long,
            value: String,
        ): String? {
            gate.await()
            authserv = value
            return value
        }

        override suspend fun setAccountColor(
            accountId: Long,
            color: Int?,
        ) {
            gate.await()
            this.color = color
        }
    }

    @Test
    fun anAuthservSaveCutShortByClosingThePageStillRefreshesTrust() {
        val core = GatedSettings()
        val trust = SenderTrustChanges()
        val store = ViewModelStore()
        val factory = viewModelFactory { initializer { ProviderAuthservViewModel(core, trust, 1) } }
        val vm = ViewModelProvider(store, factory)[ProviderAuthservViewModel::class.java]
        vm.onTextChange("mx.example.org")
        vm.save()
        store.clear() // the page closes while the write is under way
        core.gate.complete(Unit)
        assertEquals("mx.example.org", core.authserv)
        assertEquals("open messages are told the trust changed", 1L, trust.version.value)
    }

    @Test
    fun aColourSaveCutShortByClosingThePageStillRefreshesTheAccounts() {
        val core = GatedSettings()
        val store = ViewModelStore()
        val factory = viewModelFactory { initializer { AccountSettingsViewModel(core, 1) } }
        val vm = ViewModelProvider(store, factory)[AccountSettingsViewModel::class.java]
        var refreshed = 0
        vm.onColorChange(5) { refreshed++ }
        store.clear()
        core.gate.complete(Unit)
        assertEquals(5, core.color)
        assertEquals("the account list is told the colour changed", 1, refreshed)
    }
}
