package app.mailcrane.ui.message

import android.content.Context
import android.net.Uri
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.mailcrane.core.InlinePart
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.net.InetAddress
import java.net.ServerSocket
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * The message web view on a real device or emulator. A tiny HTTP server on 127.0.0.1 records every
 * request, so "no network request" is checked directly rather than assumed.
 */
@RunWith(AndroidJUnit4::class)
class SafeWebViewTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private lateinit var server: RecordingServer
    private lateinit var view: WebView

    @Before
    fun setUp() {
        server = RecordingServer()
        instrumentation.runOnMainSync { view = WebView(context) }
    }

    @After
    fun tearDown() {
        instrumentation.runOnMainSync { view.destroy() }
        server.close()
    }

    /** The client under test, also telling when a page finished loading. */
    private class Client(
        parts: Map<String, InlinePart> = emptyMap(),
        val opened: MutableList<Uri> = mutableListOf(),
        /** Test-only: let plain-http requests to the local server through when remote is allowed. */
        private val allowLocalHttp: Boolean = false,
    ) : SafeWebViewClient(inlinePart = { parts[it] }, onOpenLink = { opened += it }) {
        val requested: MutableList<Uri> = Collections.synchronizedList(mutableListOf())
        var finished = CountDownLatch(1)

        override fun shouldInterceptRequest(
            view: WebView,
            request: WebResourceRequest,
        ): WebResourceResponse? {
            requested += request.url
            if (allowLocalHttp && allowRemote && request.url.scheme == "http") return null
            return super.shouldInterceptRequest(view, request)
        }

        override fun onPageFinished(
            view: WebView,
            url: String?,
        ) {
            finished.countDown()
        }
    }

    private fun load(
        client: Client,
        html: String,
        allowRemote: Boolean,
    ) {
        client.finished = CountDownLatch(1)
        instrumentation.runOnMainSync {
            lockDown(view, allowRemote)
            client.allowRemote = allowRemote
            view.webViewClient = client
            view.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
        }
        assertTrue("page didn't load", client.finished.await(20, TimeUnit.SECONDS))
        // Give late subresource requests a chance to show up.
        Thread.sleep(1_500)
    }

    /** The document shape the core produces, with [csp] as its policy. */
    private fun document(
        body: String,
        csp: String = "default-src 'none'; img-src mailcrane-cid: data:; style-src 'unsafe-inline'",
    ) = "<!DOCTYPE html><html><head><meta charset=\"utf-8\">" +
        "<meta name=\"referrer\" content=\"no-referrer\">" +
        "<meta http-equiv=\"Content-Security-Policy\" content=\"$csp\">" +
        "</head><body>$body</body></html>"

    @Test
    fun settings_are_locked_down() {
        instrumentation.runOnMainSync {
            lockDown(view, allowRemote = false)
            with(view.settings) {
                assertFalse(javaScriptEnabled)
                assertFalse(allowFileAccess)
                assertFalse(allowContentAccess)
                assertFalse(domStorageEnabled)
                assertTrue(blockNetworkLoads)
                assertTrue(safeBrowsingEnabled)
                assertFalse(builtInZoomControls)
            }
            assertFalse(CookieManager.getInstance().acceptCookie())
            assertFalse(CookieManager.getInstance().acceptThirdPartyCookies(view))

            // Allowing remote images loosens only the network.
            lockDown(view, allowRemote = true)
            assertFalse(view.settings.javaScriptEnabled)
            assertFalse(view.settings.blockNetworkLoads)
            assertFalse(view.settings.allowFileAccess)
            assertFalse(view.settings.allowContentAccess)
        }
    }

    @Test
    fun script_and_onload_have_no_effect() {
        val client = Client()
        // Not sanitised on purpose: the web view must not run script even if the sanitiser missed it.
        load(
            client,
            "<html><head><title>start</title></head>" +
                "<body onload=\"document.title='onload'\"><script>document.title='script'</script></body></html>",
            allowRemote = false,
        )
        var title: String? = null
        instrumentation.runOnMainSync { title = view.title }
        assertEquals("start", title)
    }

    @Test
    fun no_network_request_before_remote_images_are_allowed() {
        val client = Client()
        val url = server.url("/pixel.gif")
        // Also not sanitised: every way a page could fetch something, blocked by the view itself.
        load(
            client,
            "<html><head><link rel=\"stylesheet\" href=\"${server.url("/s.css")}\">" +
                "<link rel=\"prefetch\" href=\"${server.url("/p")}\"></head><body>" +
                "<img src=\"$url\"><div style=\"background:url(${server.url("/bg.png")})\">x</div>" +
                "<iframe src=\"${server.url("/frame")}\"></iframe>" +
                "<video poster=\"${server.url("/poster.png")}\"></video></body></html>",
            allowRemote = false,
        )
        assertEquals("no request may reach the network", emptyList<String>(), server.requests())
    }

    @Test
    fun remote_images_load_without_cookies_or_referer_once_allowed() {
        val client = Client(allowLocalHttp = true)
        val url = server.url("/pixel.gif")
        // A cookie for the image's host exists; the view must not send it.
        val cookies = CookieManager.getInstance()
        instrumentation.runOnMainSync {
            cookies.setAcceptCookie(true)
            cookies.setCookie(server.url("/"), "tracking=1")
            cookies.flush()
        }
        // The core allows only https: images; https needs a trusted certificate, so this test widens
        // the policy to the local http server. The cookie and referrer behaviour is the same.
        load(
            client,
            document(
                "<a href=\"https://example.org/\"><img src=\"$url\"></a>",
                csp = "default-src 'none'; img-src http:",
            ),
            allowRemote = true,
        )
        val requests = server.requests()
        assertEquals(listOf("GET /pixel.gif"), requests.map { it.lineSequence().first().substringBeforeLast(" ") })
        val headers = requests.first().lowercase()
        assertFalse("sent a cookie: $headers", "\ncookie:" in headers)
        assertFalse("sent a referer: $headers", "\nreferer:" in headers)
        instrumentation.runOnMainSync { cookies.removeAllCookies(null) }
    }

    @Test
    fun cid_images_are_served_from_the_message_without_network() {
        val png = InlinePart("image/png", ONE_PIXEL_PNG)
        val client = Client(parts = mapOf("logo@example.org" to png))
        load(
            client,
            document("<img id=\"logo\" src=\"mailcrane-cid://part/logo%40example.org\">"),
            allowRemote = false,
        )
        assertTrue(client.requested.any { it.scheme == CID_SCHEME })
        val response = client.intercept(Uri.parse("mailcrane-cid://part/logo%40example.org"))
        assertEquals("image/png", response?.mimeType)
        assertEquals(200, response?.statusCode)
        assertEquals(404, client.intercept(Uri.parse("mailcrane-cid://part/nope%40example.org"))?.statusCode)
        assertEquals(403, client.intercept(Uri.parse("mailcrane-cid://other/logo%40example.org"))?.statusCode)
        assertEquals(403, client.intercept(Uri.parse("https://example.org/a.png"))?.statusCode)
        assertEquals(403, client.intercept(Uri.parse("file:///etc/hosts"))?.statusCode)
        assertEquals(emptyList<String>(), server.requests())
    }

    @Test
    fun links_never_navigate_inside_the_view() {
        val client = Client()
        var results = listOf<Boolean>()
        instrumentation.runOnMainSync {
            results =
                listOf(
                    "https://example.org/a",
                    "mailto:ann@example.org",
                    "javascript:alert(1)",
                    "file:///etc/hosts",
                    "intent://x#Intent;end",
                ).map { client.shouldOverrideUrlLoading(view, Request(Uri.parse(it), gesture = true)) }
            // A navigation the page starts by itself opens nothing.
            client.shouldOverrideUrlLoading(view, Request(Uri.parse("https://example.net/"), gesture = false))
        }
        assertTrue("every navigation is cancelled", results.all { it })
        assertEquals(listOf(Uri.parse("https://example.org/a"), Uri.parse("mailto:ann@example.org")), client.opened)
    }

    private class Request(
        private val uri: Uri,
        private val gesture: Boolean,
    ) : WebResourceRequest {
        override fun getUrl() = uri

        override fun isForMainFrame() = true

        override fun isRedirect() = false

        override fun hasGesture() = gesture

        override fun getMethod() = "GET"

        override fun getRequestHeaders() = emptyMap<String, String>()
    }

    /** Accepts HTTP requests on 127.0.0.1, records each request head, and answers with a GIF. */
    private class RecordingServer : AutoCloseable {
        private val socket = ServerSocket(0, 16, InetAddress.getByName("127.0.0.1"))
        private val heads = Collections.synchronizedList(mutableListOf<String>())

        init {
            thread(isDaemon = true) {
                while (!socket.isClosed) {
                    val client = runCatching { socket.accept() }.getOrNull() ?: break
                    client.use { c ->
                        val reader = c.getInputStream().bufferedReader()
                        val head =
                            generateSequence { reader.readLine() }.takeWhile { it.isNotEmpty() }.joinToString(
                                "\n",
                            )
                        heads += head
                        val out = c.getOutputStream()
                        out.write(
                            (
                                "HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\n" +
                                    "Content-Length: ${GIF.size}\r\nConnection: close\r\n\r\n"
                            ).toByteArray(),
                        )
                        out.write(GIF)
                        out.flush()
                    }
                }
            }
        }

        fun url(path: String) = "http://127.0.0.1:${socket.localPort}$path"

        fun requests(): List<String> = synchronized(heads) { heads.toList() }

        override fun close() = socket.close()
    }

    private companion object {
        val GIF: ByteArray =
            android.util.Base64.decode(
                "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
                android.util.Base64.DEFAULT,
            )
        val ONE_PIXEL_PNG: ByteArray =
            android.util.Base64.decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
                android.util.Base64.DEFAULT,
            )
    }
}
