package app.mailcrane.debug

import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent

/** Release builds have no debug tools (#289); their core has no fixture loader either. */
@Module
@InstallIn(SingletonComponent::class)
object DebugToolsModule {
    @Provides
    fun provideDebugTools(): DebugTools = NoDebugTools
}
