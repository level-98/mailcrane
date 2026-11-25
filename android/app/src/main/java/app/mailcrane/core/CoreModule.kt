package app.mailcrane.core

import android.content.Context
import app.mailcrane.contacts.AndroidDeviceContacts
import app.mailcrane.contacts.DeviceContacts
import app.mailcrane.work.OutboxSender
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

    // One instance serves both interfaces, so the message screen's changes (mark as read) reach
    // the message list through the same event listener.
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
    @Singleton
    fun provideComposeCore(
        @ApplicationContext context: Context,
        outbox: OutboxSender,
    ): ComposeCore = UniffiComposeCore(dbPath = dbPath(context), outbox = outbox)

    @Provides
    fun provideLinkCleaner(): LinkCleaner = CoreLinkCleaner

    @Provides
    fun provideDeviceContacts(contacts: AndroidDeviceContacts): DeviceContacts = contacts

    @Provides
    @Singleton
    fun provideAccountSettingsCore(
        @ApplicationContext context: Context,
    ): AccountSettingsCore = UniffiAccountSettingsCore(dbPath = dbPath(context))

    private fun dbPath(context: Context): String = databasePath(context)
}

/** The app's database: in app-private storage that is never backed up. */
fun databasePath(context: Context): String = File(context.noBackupFilesDir, "mailcrane.sqlite").path
