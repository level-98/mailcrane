package app.mailcrane.settings

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.mailcrane.BuildConfig
import app.mailcrane.R

/** The pages the About screen links to. They open in the browser, and only when tapped. */
object AboutLinks {
    const val WEBSITE = "https://mailcrane.app"
    const val PRIVACY = "https://mailcrane.app/privacy/"
    const val SOURCE = "https://github.com/mailcrane/mailcrane"
    const val LICENCE = "https://github.com/mailcrane/mailcrane/blob/main/LICENSE"
}

/** What the About screen shows about this build. */
data class AppVersion(
    val name: String,
    val code: Long,
    val commit: String,
)

/** This build's version, from the package and the commit Gradle recorded. */
internal fun appVersion(context: Context): AppVersion {
    val info =
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                context.packageManager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                context.packageManager.getPackageInfo(context.packageName, 0)
            }
        } catch (_: PackageManager.NameNotFoundException) {
            null
        }
    val code =
        when {
            info == null -> 0L
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.P -> info.longVersionCode
            else ->
                @Suppress("DEPRECATION")
                info.versionCode.toLong()
        }
    return AppVersion(name = info?.versionName.orEmpty(), code = code, commit = BuildConfig.GIT_COMMIT)
}

/** Opens [url] in the browser. Nothing happens when no browser is installed. */
internal fun openInBrowser(
    context: Context,
    url: String,
) {
    val intent =
        Intent(Intent.ACTION_VIEW, Uri.parse(url))
            .addCategory(Intent.CATEGORY_BROWSABLE)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(intent)
    } catch (_: ActivityNotFoundException) {
        // No browser; nothing to do.
    }
}

/** Settings → About Mailcrane (#75, #74). */
@Composable
fun AboutScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val version = remember { appVersion(context) }
    var showLicences by rememberSaveable { mutableStateOf(false) }
    if (showLicences) {
        BackHandler { showLicences = false }
        LicencesScreen(onBack = { showLicences = false })
        return
    }
    AboutContent(
        version = version,
        onBack = onBack,
        onOpen = { openInBrowser(context, it) },
        onOpenLicences = { showLicences = true },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AboutContent(
    version: AppVersion,
    onBack: () -> Unit,
    onOpen: (String) -> Unit,
    onOpenLicences: () -> Unit = {},
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.about_title)) },
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
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState()),
        ) {
            ListItem(
                headlineContent = { Text(stringResource(R.string.app_name)) },
                supportingContent = {
                    Column {
                        Text(
                            stringResource(R.string.about_version, version.name, version.code),
                            modifier = Modifier.testTag("about_version"),
                        )
                        Text(stringResource(R.string.about_commit, version.commit))
                    }
                },
            )
            HorizontalDivider()
            LinkRow(R.string.about_website, R.string.about_website_summary, AboutLinks.WEBSITE, onOpen)
            LinkRow(R.string.about_privacy, R.string.about_privacy_summary, AboutLinks.PRIVACY, onOpen)
            LinkRow(R.string.about_source, R.string.about_source_summary, AboutLinks.SOURCE, onOpen)
            LinkRow(R.string.about_licence, R.string.about_licence_summary, AboutLinks.LICENCE, onOpen)
            ListItem(
                headlineContent = { Text(stringResource(R.string.about_licences)) },
                supportingContent = { Text(stringResource(R.string.about_licences_summary)) },
                modifier = Modifier.clickable(role = Role.Button, onClick = onOpenLicences),
            )
            Text(
                stringResource(R.string.about_links_note),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }
    }
}

@Composable
private fun LinkRow(
    @StringRes title: Int,
    @StringRes summary: Int,
    url: String,
    onOpen: (String) -> Unit,
) {
    ListItem(
        headlineContent = { Text(stringResource(title)) },
        supportingContent = { Text(stringResource(summary)) },
        modifier = Modifier.clickable(role = Role.Button, onClick = { onOpen(url) }),
    )
}
