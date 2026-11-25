package app.mailcrane.oauth

import android.app.Activity
import android.os.Bundle
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
 */
class OAuthRedirectActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uri = intent?.data
        if (OAuthSignIn.isRedirect(uri)) {
            val redirect = uri.toString()
            scope.launch {
                val result =
                    try {
                        OAuthResult.SignedIn(oauthComplete(redirect))
                    } catch (e: OAuthFailure) {
                        OAuthResult.Failed(e)
                    }
                OAuthSignIn.report(result)
            }
        }
        finish()
    }

    private companion object {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
