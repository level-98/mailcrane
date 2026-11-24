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
    ): MailcraneCore =
        // The database lives in app-private storage that is never backed up.
        UniffiMailcraneCore(dbPath = File(context.noBackupFilesDir, "mailcrane.sqlite").path)
}
