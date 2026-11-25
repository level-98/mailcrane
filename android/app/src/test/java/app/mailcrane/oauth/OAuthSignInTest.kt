package app.mailcrane.oauth

import android.app.Application
import android.net.Uri
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the app's own loads the native core, which unit tests don't have.
@Config(sdk = [35], application = Application::class)
class OAuthSignInTest {
    @Test
    fun onlyOurRedirectIsAccepted() {
        assertTrue(OAuthSignIn.isRedirect(Uri.parse("app.mailcrane:/oauth2redirect?code=a&state=b")))
        assertFalse(OAuthSignIn.isRedirect(Uri.parse("app.mailcrane:/other?code=a")))
        assertFalse(OAuthSignIn.isRedirect(Uri.parse("https://example.org/oauth2redirect?code=a")))
        assertFalse(OAuthSignIn.isRedirect(null))
    }

    @Test
    fun theRedirectUriIsAPrivateUseScheme() {
        // RFC 8252 §7.1: reverse domain of the app, single slash.
        assertTrue(OAuthSignIn.isRedirect(Uri.parse(OAuthSignIn.REDIRECT_URI)))
    }
}
