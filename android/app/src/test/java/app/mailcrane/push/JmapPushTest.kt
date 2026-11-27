package app.mailcrane.push

import android.app.Application
import androidx.work.ListenableWorker
import androidx.work.NetworkType
import app.mailcrane.core.JmapPushCore
import app.mailcrane.work.JmapPushOp
import app.mailcrane.work.JmapPushWorker
import app.mailcrane.work.runJmapPush
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.concurrent.TimeUnit

/** JMAP push subscriptions on Android (M4-27): keys, routing, and what the worker asks the core. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class JmapPushTest {
    private val context: Application = RuntimeEnvironment.getApplication()

    private class FakeCore(
        var answers: Boolean = true,
    ) : JmapPushCore {
        val registered = mutableListOf<Triple<String, String?, String?>>()
        val answered = mutableListOf<String>()
        var unregistered = 0

        override suspend fun registerPush(
            endpoint: String,
            p256dh: String?,
            auth: String?,
        ): Int {
            registered += Triple(endpoint, p256dh, auth)
            return 1
        }

        override suspend fun answerPush(body: ByteArray): Boolean {
            answered += body.decodeToString()
            return answers
        }

        override suspend fun unregisterPush() {
            unregistered++
        }
    }

    private val ready =
        PushStatus.Ready(
            "https://push.example.org/up/1",
            temporary = false,
            keys = WebPushKeys("BPublicKeyExample", "authSecretExample"),
        )
    private val verification =
        """{"@type":"PushVerification","pushSubscriptionId":"P1","verificationCode":"c"}""".encodeToByteArray()

    @Before
    fun setUp() = UnifiedPushState.forgetForTests()

    @After
    fun tearDown() = UnifiedPushState.forgetForTests()

    @Test
    fun theKeysSurviveAProcessRestart() {
        UnifiedPushState.get(context).set(ready)
        UnifiedPushState.forgetForTests()
        assertEquals(ready, UnifiedPushState.get(context).status.value)
        // Without keys from the distributor, none are invented.
        UnifiedPushState.get(context).set(PushStatus.Ready("https://push.example.org/up/2", temporary = false))
        UnifiedPushState.forgetForTests()
        assertNull((UnifiedPushState.get(context).status.value as PushStatus.Ready).keys)
    }

    @Test
    fun verificationsAreAnsweredAndEverythingElseWakes() {
        val verified = mutableListOf<ByteArray>()
        var woken = 0
        val isVerification = { b: ByteArray -> b.decodeToString().contains("PushVerification") }
        routePush(verification, isVerification, { verified += it }, { woken++ })
        routePush("{}".encodeToByteArray(), isVerification, { verified += it }, { woken++ })
        routePush(ByteArray(0), isVerification, { verified += it }, { woken++ })
        assertEquals(1, verified.size)
        assertEquals(2, woken)
    }

    @Test
    fun registerSendsTheEndpointAndKeysOnlyWhileReady() =
        runTest {
            val core = FakeCore()
            assertEquals(ListenableWorker.Result.success(), runJmapPush(JmapPushOp.REGISTER, ready, null, core, 0))
            assertEquals(listOf(Triple(ready.endpoint, "BPublicKeyExample", "authSecretExample")), core.registered)
            // Turned off, or still waiting for the distributor: nothing is registered.
            runJmapPush(JmapPushOp.REGISTER, PushStatus.Off, null, core, 0)
            runJmapPush(JmapPushOp.REGISTER, PushStatus.Registering, null, core, 0)
            assertEquals(1, core.registered.size)
        }

    @Test
    fun aFailedVerificationIsRetriedAFewTimes() =
        runTest {
            val core = FakeCore(answers = true)
            assertEquals(
                ListenableWorker.Result.success(),
                runJmapPush(JmapPushOp.VERIFY, ready, verification, core, 0),
            )
            core.answers = false
            assertEquals(ListenableWorker.Result.retry(), runJmapPush(JmapPushOp.VERIFY, ready, verification, core, 0))
            assertEquals(ListenableWorker.Result.retry(), runJmapPush(JmapPushOp.VERIFY, ready, verification, core, 2))
            assertEquals(
                ListenableWorker.Result.success(),
                runJmapPush(JmapPushOp.VERIFY, ready, verification, core, 3),
            )
            // Push turned off meanwhile: not answered.
            runJmapPush(JmapPushOp.VERIFY, PushStatus.Off, verification, core, 0)
            assertEquals(4, core.answered.size)
        }

    @Test
    fun turningOffUnregistersWhateverTheState() =
        runTest {
            val core = FakeCore()
            runJmapPush(JmapPushOp.UNREGISTER, PushStatus.Off, null, core, 0)
            assertEquals(1, core.unregistered)
        }

    @Test
    fun theRequestsAreShapedForTheirJob() {
        val verify = JmapPushWorker.verifyRequest(verification)
        assertTrue(verify?.workSpec?.expedited == true)
        assertEquals(NetworkType.CONNECTED, verify?.workSpec?.constraints?.requiredNetworkType)
        // Too big to be a verification, or to fit in work data: not queued.
        assertNull(JmapPushWorker.verifyRequest(ByteArray(JmapPushWorker.MAX_VERIFY_BYTES + 1)))
        val refresh = JmapPushWorker.refreshRequest().workSpec
        assertEquals(TimeUnit.DAYS.toMillis(1), refresh.intervalDuration)
        assertEquals(NetworkType.CONNECTED, refresh.constraints.requiredNetworkType)
        assertEquals(
            NetworkType.CONNECTED,
            JmapPushWorker
                .unregisterRequest()
                .workSpec.constraints.requiredNetworkType,
        )
    }
}
