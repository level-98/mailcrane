package app.mailcrane.oauth

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import app.mailcrane.BuildConfig
import app.mailcrane.core.OAuthFailure
import app.mailcrane.core.OAuthProviderConfig
import app.mailcrane.core.oauthBegin
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

/** How a sign-in in the browser ended (M3-01). */
sealed interface OAuthResult {
    /** The browser came back with a redirect; the code is being exchanged. A result follows. */
    data object Returned : OAuthResult

    /** Signed in: pass [signInId] to `oauthAttach` once the account is saved. */
    data class SignedIn(
        val signInId: String,
    ) : OAuthResult

    data class Failed(
        val reason: OAuthFailure?,
    ) : OAuthResult
}

/**
 * Sign-in with OAuth in the system browser (RFC 8252): a Custom Tab, or the default browser if none
 * supports them. Never an in-app web view, so the app never sees the user's provider password.
 * The redirect comes back to [OAuthRedirectActivity], which reports on [results].
 */
object OAuthSignIn {
    /**
     * The redirect URI every provider is registered with: a private-use scheme (RFC 8252 §7.1), the
     * release package. A literal, the same as the core's (`providers/google.rs`): the build's
     * CheckOAuthRedirect compares the two and checks that only release builds route it.
     */
    const val REDIRECT_URI = "app.mailcrane:/oauth2redirect"

    /** Whether this build offers Sign in with Google: release only, see [REDIRECT_URI] (#405). */
    val AVAILABLE: Boolean = BuildConfig.GOOGLE_SIGN_IN

    private val resultsFlow = MutableSharedFlow<OAuthResult>(extraBufferCapacity = 4)
    val results: SharedFlow<OAuthResult> = resultsFlow.asSharedFlow()

    internal fun report(result: OAuthResult) {
        resultsFlow.tryEmit(result)
    }

    /** Opens the provider's sign-in page. Returns false if no browser could be opened. */
    fun start(
        context: Context,
        provider: OAuthProviderConfig,
        loginHint: String?,
    ): Boolean {
        val url =
            try {
                Uri.parse(oauthBegin(provider, loginHint).url)
            } catch (e: OAuthFailure) {
                report(OAuthResult.Failed(e))
                return false
            }
        return try {
            CustomTabsIntent.Builder().build().launchUrl(context, url)
            true
        } catch (_: ActivityNotFoundException) {
            try {
                context.startActivity(
                    Intent(Intent.ACTION_VIEW, url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
                true
            } catch (_: ActivityNotFoundException) {
                false
            }
        }
    }

    /** Whether [uri] is a redirect to us at all (the core checks the rest, `state` first). */
    fun isRedirect(uri: Uri?): Boolean =
        uri != null && uri.scheme == REDIRECT_URI.substringBefore(':') && uri.path == REDIRECT_URI.substringAfter(':')
}
