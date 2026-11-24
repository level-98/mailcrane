package app.mailcrane.core

import android.content.Context
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import java.io.File
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
object CoreModule {
    // The database lives in app-private storage that is never backed up.
    private fun dbPath(context: Context): String = File(context.noBackupFilesDir, "mailcrane.sqlite").path

    @Provides
    @Singleton
    fun provideMailcraneCore(
        @ApplicationContext context: Context,
    ): MailcraneCore = UniffiMailcraneCore(dbPath = dbPath(context))

    // One sync engine for the app: live updates send their events to the same listeners as the
    // screens' own syncs.
    @Provides
    @Singleton
    fun provideUniffiMailboxCore(
        @ApplicationContext context: Context,
    ): UniffiMailboxCore = UniffiMailboxCore(dbPath = dbPath(context))

    @Provides
    fun provideMailboxCore(core: UniffiMailboxCore): MailboxCore = core

    @Provides
    fun provideLiveUpdatesEngine(core: UniffiMailboxCore): LiveUpdatesEngine = core
}
