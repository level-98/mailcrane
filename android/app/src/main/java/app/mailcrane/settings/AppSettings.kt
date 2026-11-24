package app.mailcrane.settings

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.preferencesDataStore
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import java.io.IOException
import javax.inject.Singleton

/**
 * The user's app settings. An interface so view models can be tested with a fake. Settings hold
 * preferences only, never secrets (those go to the Keystore through the secret store).
 */
interface AppSettings {
    /** "Remove tracking from links": strip tracking parameters before opening a link. On by default. */
    val cleanLinks: Flow<Boolean>

    suspend fun setCleanLinks(enabled: Boolean)
}

private val Context.settingsDataStore: DataStore<Preferences> by preferencesDataStore(name = "settings")

/** [AppSettings] kept in a Preferences DataStore. */
class DataStoreAppSettings(
    private val dataStore: DataStore<Preferences>,
) : AppSettings {
    // An unreadable settings file means the defaults, not a crash.
    private val prefs: Flow<Preferences> =
        dataStore.data.catch { e -> if (e is IOException) emit(emptyPreferences()) else throw e }

    override val cleanLinks: Flow<Boolean> = prefs.map { it[CLEAN_LINKS] ?: true }

    override suspend fun setCleanLinks(enabled: Boolean) {
        dataStore.edit { it[CLEAN_LINKS] = enabled }
    }

    private companion object {
        val CLEAN_LINKS = booleanPreferencesKey("clean_links")
    }
}

@Module
@InstallIn(SingletonComponent::class)
object SettingsModule {
    @Provides
    @Singleton
    fun provideAppSettings(
        @ApplicationContext context: Context,
    ): AppSettings = DataStoreAppSettings(context.settingsDataStore)
}
