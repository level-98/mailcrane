package app.mailcrane.ui.message

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.AuthResult
import app.mailcrane.core.AuthVerdict
import app.mailcrane.core.SenderCheck

/** How each verdict looks: the words carry the meaning, the colour only repeats it. */
private data class VerdictLook(
    @StringRes val label: Int,
    @StringRes val explain: Int,
    @DrawableRes val icon: Int,
)

private fun look(verdict: AuthVerdict): VerdictLook =
    when (verdict) {
        AuthVerdict.VERIFIED ->
            VerdictLook(R.string.message_auth_verified, R.string.message_auth_explain_verified, R.drawable.ic_mc_shield)
        AuthVerdict.NOT_VERIFIED ->
            VerdictLook(
                R.string.message_auth_not_verified,
                R.string.message_auth_explain_not_verified,
                R.drawable.ic_mc_shield,
            )
        AuthVerdict.FAILED ->
            VerdictLook(R.string.message_auth_failed, R.string.message_auth_explain_failed, R.drawable.ic_mc_error)
    }

@Composable
private fun tint(verdict: AuthVerdict): Color =
    when (verdict) {
        AuthVerdict.VERIFIED -> MaterialTheme.colorScheme.primary
        AuthVerdict.NOT_VERIFIED -> MaterialTheme.colorScheme.onSurfaceVariant
        AuthVerdict.FAILED -> MaterialTheme.colorScheme.error
    }

/**
 * The authentication chip next to the sender (M5-07). Shows nothing when the provider's
 * Authentication-Results header wasn't found: most servers that don't add one are fine.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SenderAuthChip(check: SenderCheck?) {
    val verdict = check?.verdict ?: return
    var open by rememberSaveable { mutableStateOf(false) }
    val look = look(verdict)
    val color = tint(verdict)
    AssistChip(
        onClick = { open = true },
        label = { Text(stringResource(look.label)) },
        leadingIcon = {
            Icon(
                painter = painterResource(look.icon),
                contentDescription = null,
                modifier = Modifier.size(AssistChipDefaults.IconSize),
            )
        },
        colors = AssistChipDefaults.assistChipColors(labelColor = color, leadingIconContentColor = color),
        modifier = Modifier.testTag("sender-auth-chip"),
    )
    if (open) {
        ModalBottomSheet(onDismissRequest = { open = false }) {
            SenderAuthSheet(check)
        }
    }
}

/** SPF, DKIM and DMARC, one per row, in plain words. */
@Composable
internal fun SenderAuthSheet(check: SenderCheck) {
    val verdict = check.verdict ?: return
    val look = look(verdict)
    Column(Modifier.navigationBarsPadding().padding(bottom = 16.dp).testTag("sender-auth-sheet")) {
        Text(
            text = stringResource(look.label),
            style = MaterialTheme.typography.titleLarge,
            color = tint(verdict),
            modifier = Modifier.padding(horizontal = 24.dp).semantics { heading() },
        )
        Text(
            text = stringResource(look.explain),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
        )
        AuthRow(R.string.message_auth_spf, check.spf, check.spfDomain)
        AuthRow(R.string.message_auth_dkim, check.dkim, check.dkimDomain)
        AuthRow(R.string.message_auth_dmarc, check.dmarc, check.dmarcDomain)
        Text(
            text = stringResource(R.string.message_auth_footnote),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
        )
    }
}

@Composable
private fun AuthRow(
    @StringRes name: Int,
    result: AuthResult,
    domain: String?,
) {
    val words = stringResource(resultText(result))
    ListItem(
        headlineContent = { Text(stringResource(name)) },
        supportingContent = {
            Text(if (domain.isNullOrEmpty()) words else stringResource(R.string.message_auth_result_domain, words, domain))
        },
    )
}

@StringRes
private fun resultText(result: AuthResult): Int =
    when (result) {
        AuthResult.PASS -> R.string.message_auth_result_pass
        AuthResult.FAIL -> R.string.message_auth_result_fail
        AuthResult.NEUTRAL -> R.string.message_auth_result_neutral
        AuthResult.NONE -> R.string.message_auth_result_none
        AuthResult.TEMP_ERROR -> R.string.message_auth_result_temperror
        AuthResult.PERM_ERROR -> R.string.message_auth_result_permerror
        AuthResult.UNKNOWN -> R.string.message_auth_result_unknown
    }
