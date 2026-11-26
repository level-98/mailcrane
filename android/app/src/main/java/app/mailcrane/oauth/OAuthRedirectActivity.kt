package app.mailcrane.oauth

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import app.mailcrane.MainActivity
import app.mailcrane.core.OAuthFailure
import app.mailcrane.core.oauthComplete
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Receives the browser's redirect after sign-in (M3-01) and hands it to the core, which checks
 * `state` before anything else and exchanges the code. It must be exported for the browser to
 * reach it, so it trusts nothing in the intent: a forged redirect has no matching sign-in and is
 * refused without any request. Shows nothing and finishes at once.
 *
 * It brings the app back above the browser tab: the Custom Tab was opened in the app's task, so
 * without this the user was left in it after approving (#376).
 */
class OAuthRedirectActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uri = intent?.data
        if (OAuthSignIn.isRedirect(uri)) {
            val redirect = uri.toString()
            OAuthSignIn.report(OAuthResult.Returned)
            startActivity(returnToApp(this))
            scope.launch {
                val result =
                    try {
                        OAuthResult.SignedIn(complete(redirect))
                    } catch (e: OAuthFailure) {
                        OAuthResult.Failed(e)
                    }
                OAuthSignIn.report(result)
            }
        }
        finish()
    }

    internal companion object {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

        /** Exchanges the code; replaced in tests, which have no native core. */
        @Volatile var complete: suspend (String) -> String = { oauthComplete(it) }

        /**
         * The app's existing screen, brought to the front with everything above it (the browser
         * tab) cleared, and not recreated, so the setup screen and its state are still there.
         */
        fun returnToApp(context: Context): Intent =
            Intent(context, MainActivity::class.java).addFlags(
                Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP,
            )
    }
}
