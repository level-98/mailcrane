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

    // One instance serves every interface: the message screen's changes (mark as read) and live
    // updates reach the message list through the same event listener.
    @Provides
    @Singleton
    fun provideUniffiMailboxCore(
        @ApplicationContext context: Context,
    ): UniffiMailboxCore = UniffiMailboxCore(dbPath = dbPath(context))

    @Provides
    fun provideMailboxCore(core: UniffiMailboxCore): MailboxCore = core

    @Provides
    fun provideMessageCore(core: UniffiMailboxCore): MessageCore = core

    @Provides
    fun provideLiveUpdatesEngine(core: UniffiMailboxCore): LiveUpdatesEngine = core
}
