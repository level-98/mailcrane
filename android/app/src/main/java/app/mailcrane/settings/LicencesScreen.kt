package app.mailcrane.settings

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException

/** One library the app ships, from assets/licences.json (#366). */
data class OssLibrary(
    val name: String,
    val version: String,
    /** "rust" for a crate in the core, "android" for a Java or Kotlin library. */
    val kind: String,
    /** SPDX licence id, e.g. Apache-2.0. */
    val licence: String,
    /** Key of its licence text in [OssLicences.texts]. */
    val textKey: String,
)

/** The bundled notices: every library and the licence texts they use. */
data class OssLicences(
    val libraries: List<OssLibrary>,
    val texts: Map<String, String>,
) {
    companion object {
        val EMPTY = OssLicences(emptyList(), emptyMap())

        /** Parses the file the build writes. Anything malformed gives an empty list, never a crash. */
        fun parse(json: String): OssLicences =
            try {
                val root = JSONObject(json)
                val textsJson = root.getJSONObject("texts")
                val texts =
                    textsJson.keys().asSequence().associateWith { key ->
                        textsJson.getJSONObject(key).optString("text")
                    }
                val array = root.getJSONArray("libraries")
                val libraries =
                    (0 until array.length()).map { i ->
                        val o = array.getJSONObject(i)
                        OssLibrary(
                            name = o.optString("name"),
                            version = o.optString("version"),
                            kind = o.optString("kind"),
                            licence = o.optString("licence"),
                            textKey = o.optString("text"),
                        )
                    }
                OssLicences(libraries, texts)
            } catch (_: JSONException) {
                EMPTY
            }

        /** Reads assets/licences.json. */
        fun load(context: Context): OssLicences =
            try {
                context.assets
                    .open("licences.json")
                    .bufferedReader()
                    .use { parse(it.readText()) }
            } catch (_: IOException) {
                EMPTY
            }
    }
}

/** About → Open-source licences. */
@Composable
fun LicencesScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val licences = remember { OssLicences.load(context) }
    LicencesContent(licences, onBack)
}

@Composable
internal fun LicencesContent(
    licences: OssLicences,
    onBack: () -> Unit,
) {
    var openIndex by rememberSaveable { mutableStateOf(-1) }
    val open = licences.libraries.getOrNull(openIndex)
    if (open != null) {
        BackHandler { openIndex = -1 }
        LicenceText(open, licences.texts[open.textKey].orEmpty(), onBack = { openIndex = -1 })
        return
    }
    LicencesScaffold(stringResource(R.string.about_licences), onBack) { modifier ->
        LazyColumn(modifier) {
            item {
                Text(
                    stringResource(R.string.about_licences_intro, licences.libraries.size),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }
            items(licences.libraries.size) { i ->
                val lib = licences.libraries[i]
                ListItem(
                    headlineContent = { Text(lib.name) },
                    supportingContent = {
                        Text(
                            stringResource(
                                if (lib.kind == "rust") R.string.about_licence_rust else R.string.about_licence_android,
                                lib.version,
                                lib.licence,
                            ),
                        )
                    },
                    modifier = Modifier.clickable(role = Role.Button, onClick = { openIndex = i }),
                )
            }
        }
    }
}

@Composable
private fun LicenceText(
    library: OssLibrary,
    text: String,
    onBack: () -> Unit,
) {
    LicencesScaffold(library.name, onBack) { modifier ->
        SelectionContainer(modifier.verticalScroll(rememberScrollState())) {
            Text(
                text,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(16.dp),
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LicencesScaffold(
    title: String,
    onBack: () -> Unit,
    content: @Composable (Modifier) -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.settings_back),
                        )
                    }
                },
            )
        },
    ) { padding -> content(Modifier.fillMaxSize().padding(padding)) }
}
