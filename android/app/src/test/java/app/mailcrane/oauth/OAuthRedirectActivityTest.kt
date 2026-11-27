package app.mailcrane.oauth

import android.app.Application
import android.content.Intent
import android.net.Uri
import app.mailcrane.MainActivity
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** #376: after approving in the browser, the user lands back in the app, not in the browser tab. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the app's own loads the native core, which unit tests don't have.
@Config(sdk = [35], application = Application::class)
class OAuthRedirectActivityTest {
    private val original = OAuthRedirectActivity.complete

    @After
    fun tearDown() {
        OAuthRedirectActivity.complete = original
    }

    @Test
    fun theRedirectBringsTheAppBackAboveTheBrowserAndReportsTheSignIn() =
        runBlocking {
            OAuthRedirectActivity.complete = { "signin-9" }
            val results = async(start = CoroutineStart.UNDISPATCHED) { OAuthSignIn.results.take(2).toList() }
            val redirect = Uri.parse("app.mailcrane:/oauth2redirect?code=abc&state=xyz")
            val controller =
                Robolectric.buildActivity(OAuthRedirectActivity::class.java, Intent(Intent.ACTION_VIEW, redirect))
            val activity = controller.create().get()

            val started = shadowOf(activity).nextStartedActivity
            assertEquals(MainActivity::class.java.name, started.component?.className)
            val flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
            assertEquals(flags, started.flags and flags)
            assertTrue(activity.isFinishing)
            assertEquals(
                listOf(OAuthResult.Returned, OAuthResult.SignedIn("signin-9")),
                withTimeout(5_000) { results.await() },
            )
        }

    @Test
    fun anythingElseStartsNothing() {
        val other = Uri.parse("https://example.org/oauth2redirect?code=abc")
        val activity =
            Robolectric
                .buildActivity(OAuthRedirectActivity::class.java, Intent(Intent.ACTION_VIEW, other))
                .create()
                .get()
        assertNull(shadowOf(activity).nextStartedActivity)
        assertTrue(activity.isFinishing)
    }
}
