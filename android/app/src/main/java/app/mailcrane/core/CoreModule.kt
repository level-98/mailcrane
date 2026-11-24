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
    @Provides
    @Singleton
    fun provideMailcraneCore(
        @ApplicationContext context: Context,
    ): MailcraneCore = UniffiMailcraneCore(dbPath = dbPath(context))

    @Provides
    @Singleton
    fun provideMailboxCore(
        @ApplicationContext context: Context,
    ): MailboxCore = UniffiMailboxCore(dbPath = dbPath(context))

    @Provides
    @Singleton
    fun provideComposeCore(
        @ApplicationContext context: Context,
    ): ComposeCore = UniffiComposeCore(dbPath = dbPath(context))

    // The database lives in app-private storage that is never backed up.
    private fun dbPath(context: Context): String = File(context.noBackupFilesDir, "mailcrane.sqlite").path
}
