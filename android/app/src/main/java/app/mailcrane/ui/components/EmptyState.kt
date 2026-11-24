package app.mailcrane.ui.components

import android.content.res.Configuration
import androidx.annotation.DrawableRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.ui.theme.MailcraneTheme

/**
 * A screen with nothing to show: a brand illustration, a [title], a [body] and an optional
 * [action] (usually a button).
 *
 * Fills the space it's given and centres its content. When the content is taller than the space
 * (large text, landscape) it scrolls instead of clipping, which also lets a surrounding
 * pull-to-refresh work. The illustration is decorative, so screen readers read only the text.
 */
@Composable
fun EmptyState(
    @DrawableRes illustration: Int,
    title: String,
    body: String,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null,
) {
    BoxWithConstraints(modifier = modifier.fillMaxSize()) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = maxHeight)
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 32.dp, vertical = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Image(
                painter = painterResource(illustration),
                contentDescription = null,
                modifier =
                    Modifier
                        .widthIn(max = ILLUSTRATION_MAX_WIDTH)
                        .fillMaxWidth()
                        .aspectRatio(ILLUSTRATION_ASPECT_RATIO),
            )
            Spacer(Modifier.height(24.dp))
            Text(
                text = title,
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurface,
                textAlign = TextAlign.Center,
                modifier = Modifier.semantics { heading() },
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = body,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = 360.dp),
            )
            if (action != null) {
                Spacer(Modifier.height(24.dp))
                action()
            }
        }
    }
}

/** A folder with no messages. */
@Composable
fun EmptyFolderState(modifier: Modifier = Modifier) {
    EmptyState(
        illustration = R.drawable.illustration_empty_inbox,
        title = stringResource(R.string.empty_folder_title),
        body = stringResource(R.string.empty_folder_body),
        modifier = modifier,
    )
}

/** A search that found nothing. For search (M4-03). */
@Composable
fun NoResultsState(modifier: Modifier = Modifier) {
    EmptyState(
        illustration = R.drawable.illustration_no_results,
        title = stringResource(R.string.no_results_title),
        body = stringResource(R.string.no_results_body),
        modifier = modifier,
    )
}

/** The start screen when no account has been added yet. */
@Composable
fun WelcomeState(
    onAddAccount: () -> Unit,
    modifier: Modifier = Modifier,
) {
    EmptyState(
        illustration = R.drawable.illustration_welcome,
        title = stringResource(R.string.welcome_title),
        body = stringResource(R.string.welcome_body),
        modifier = modifier,
        action = {
            Button(onClick = onAddAccount) { Text(stringResource(R.string.welcome_add_account)) }
        },
    )
}

// The illustrations are drawn on a 240 × 200 canvas (design/build.py).
private val ILLUSTRATION_MAX_WIDTH = 240.dp
private const val ILLUSTRATION_ASPECT_RATIO = 240f / 200f

@Composable
private fun PreviewSurface(content: @Composable () -> Unit) {
    MailcraneTheme {
        Surface(color = MaterialTheme.colorScheme.surface, content = content)
    }
}

@Preview(name = "Light", showBackground = true)
@Preview(name = "Dark", showBackground = true, uiMode = Configuration.UI_MODE_NIGHT_YES)
@Preview(name = "Large text", showBackground = true, fontScale = 2f)
@Composable
private fun EmptyFolderStatePreview() {
    PreviewSurface { EmptyFolderState() }
}

@Preview(name = "Light", showBackground = true)
@Preview(name = "Dark", showBackground = true, uiMode = Configuration.UI_MODE_NIGHT_YES)
@Preview(name = "Large text", showBackground = true, fontScale = 2f)
@Composable
private fun NoResultsStatePreview() {
    PreviewSurface { NoResultsState() }
}

@Preview(name = "Light", showBackground = true)
@Preview(name = "Dark", showBackground = true, uiMode = Configuration.UI_MODE_NIGHT_YES)
@Preview(name = "Large text", showBackground = true, fontScale = 2f)
@Composable
private fun WelcomeStatePreview() {
    PreviewSurface { WelcomeState(onAddAccount = {}) }
}
