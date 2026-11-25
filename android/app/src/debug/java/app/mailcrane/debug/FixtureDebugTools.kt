package app.mailcrane.debug

import android.content.Context
import android.content.res.AssetManager
import app.mailcrane.core.FixtureFile
import app.mailcrane.core.databasePath
import app.mailcrane.core.loadTestMail
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import javax.inject.Inject

/** Where the build copies `tests/fixtures` (`.eml` and `.html` only) in the debug assets. */
private const val FIXTURES_ROOT = "fixtures"

/**
 * Debug builds only (#289): "Load test mail" hands every fixture in the debug assets to the core's
 * `load_test_mail`, which exists only in the debug core. The fixtures are synthetic test data; this
 * passes them on as bytes and never looks inside.
 */
class FixtureDebugTools
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
    ) : DebugTools {
        override val available: Boolean = true

        override suspend fun loadFixtures(): Int =
            withContext(Dispatchers.IO) {
                val files = mutableListOf<FixtureFile>()
                collect(context.assets, FIXTURES_ROOT, files)
                loadTestMail(databasePath(context), files).imported.toInt()
            }

        private fun collect(
            assets: AssetManager,
            dir: String,
            out: MutableList<FixtureFile>,
        ) {
            for (name in assets.list(dir).orEmpty().sorted()) {
                val path = "$dir/$name"
                // A directory lists its children; a file lists nothing.
                if (assets.list(path).orEmpty().isNotEmpty()) {
                    collect(assets, path, out)
                } else {
                    val bytes = assets.open(path).use { it.readBytes() }
                    out += FixtureFile(path.removePrefix("$FIXTURES_ROOT/"), bytes)
                }
            }
        }
    }

@Module
@InstallIn(SingletonComponent::class)
abstract class DebugToolsModule {
    @Binds
    abstract fun bindDebugTools(tools: FixtureDebugTools): DebugTools
}
