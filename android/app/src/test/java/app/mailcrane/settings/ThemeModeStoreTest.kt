package app.mailcrane.settings

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.emptyPreferences
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

/** #339: the chosen theme shows at once, without waiting for the settings file. */
@OptIn(ExperimentalCoroutinesApi::class)
class ThemeModeStoreTest {
    /** A settings file whose writes wait for [release], like a slow disk. */
    private class SlowDataStore : DataStore<Preferences> {
        val release = CompletableDeferred<Unit>()
        var fail = false
        override val data = MutableStateFlow(emptyPreferences())

        override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences {
            release.await()
            if (fail) throw java.io.IOException("disk full")
            val next = transform(data.value)
            data.value = next
            return next
        }
    }

    @Test
    fun theChoiceShowsBeforeTheWriteFinishes() =
        runTest {
            val store = SlowDataStore()
            val settings = DataStoreAppSettings(store)
            assertEquals(ThemeMode.SYSTEM, settings.themeMode.first())
            val write = launch { settings.setThemeMode(ThemeMode.DARK) }
            runCurrent()
            assertEquals("shown while the file is written", ThemeMode.DARK, settings.themeMode.first())
            store.release.complete(Unit)
            write.join()
            assertEquals(ThemeMode.DARK, settings.themeMode.first())
        }

    @Test
    fun aFailedWriteFallsBackToTheStoredChoice() =
        runTest {
            val store = SlowDataStore().apply { fail = true }
            val settings = DataStoreAppSettings(store)
            val write = launch { runCatching { settings.setThemeMode(ThemeMode.DARK) } }
            runCurrent()
            assertEquals(ThemeMode.DARK, settings.themeMode.first())
            store.release.complete(Unit)
            write.join()
            assertEquals(ThemeMode.SYSTEM, settings.themeMode.first())
        }
}
