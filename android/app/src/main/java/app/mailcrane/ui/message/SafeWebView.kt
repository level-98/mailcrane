package app.mailcrane.ui.message

import android.annotation.SuppressLint
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.viewinterop.AndroidView
import app.mailcrane.core.InlinePart
import java.io.ByteArrayInputStream

/** The scheme the core rewrites `cid:` images to (`mailcrane-cid://part/<content-id>`). */
const val CID_SCHEME = "mailcrane-cid"
private const val CID_HOST = "part"

/** Link schemes opened outside the app when tapped. Everything else does nothing. */
private val EXTERNAL_LINK_SCHEMES = setOf("https", "http", "mailto")

/**
 * Shows sanitised message HTML (a complete document from the core) in a web view that can't run
 * JavaScript, can't read files, keeps no cookies, never navigates, and makes no network request
 * unless [allowRemote]; even then only `https:` images load, as the document's CSP allows.
 * `cid:` images come from [inlinePart]. Tapped links go to [onOpenLink].
 */
@Composable
fun SafeWebView(
    html: String,
    allowRemote: Boolean,
    inlinePart: (contentId: String) -> InlinePart?,
    onOpenLink: (Uri) -> Unit,
    modifier: Modifier = Modifier,
    darken: Boolean = false,
) {
    val fontScale = LocalDensity.current.fontScale
    val currentInlinePart by rememberUpdatedState(inlinePart)
    val currentOpenLink by rememberUpdatedState(onOpenLink)
    AndroidView(
        modifier = modifier,
        factory = { context ->
            WebView(context).apply {
                fitToWidth(this)
                layoutParams =
                    ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
                // Mail is designed for a white page, whatever the app theme.
                setBackgroundColor(Color.WHITE)
                webViewClient =
                    SafeWebViewClient(
                        inlinePart = { currentInlinePart(it) },
                        onOpenLink = { currentOpenLink(it) },
                    )
            }
        },
        update = { view ->
            lockDown(view, allowRemote, fontScale)
            // "Dark message bodies": the system darkens the page (Android 13+; the activity theme is dark).
            // Otherwise mail keeps the white page it was designed for.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                view.settings.isAlgorithmicDarkeningAllowed = darken
            }
            view.setBackgroundColor(if (darken) Color.TRANSPARENT else Color.WHITE)
            (view.webViewClient as? SafeWebViewClient)?.allowRemote = allowRemote
            if (view.tag != html) {
                view.tag = html
                // No base URL: the page has an opaque origin and nothing to resolve relative URLs against.
                view.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
            }
        },
        onRelease = { it.destroy() },
    )
}

/**
 * Shows mail laid out wider than the screen (fixed-width newsletters, often 600 px) zoomed out to the
 * screen's width instead of cut off on the right (#342). Layout only: no setting that [lockDown]
 * applies changes, and zooming by hand stays off.
 */
fun fitToWidth(view: WebView) {
    view.settings.apply {
        // Honour the document's viewport (the device width) and shrink wider content to fit it.
        useWideViewPort = true
        loadWithOverviewMode = true
    }
}

/**
 * Applies every safety setting. Called whenever [allowRemote] or the font scale changes, so it only
 * ever loosens network access, never the rest.
 */
@SuppressLint("SetJavaScriptEnabled") // Sets it to false.
fun lockDown(
    view: WebView,
    allowRemote: Boolean,
    fontScale: Float = 1f,
) {
    view.settings.apply {
        javaScriptEnabled = false
        javaScriptCanOpenWindowsAutomatically = false
        allowFileAccess = false
        allowContentAccess = false
        domStorageEnabled = false
        databaseEnabled = false
        setGeolocationEnabled(false)
        setSupportMultipleWindows(false)
        mediaPlaybackRequiresUserGesture = true
        blockNetworkLoads = !allowRemote
        // Not blockNetworkImage: it also stops `mailcrane-cid:` images. Remote images are already
        // stopped by blockNetworkLoads, the request filter and the page's CSP.
        blockNetworkImage = false
        safeBrowsingEnabled = true
        cacheMode = WebSettings.LOAD_NO_CACHE
        setSupportZoom(false)
        builtInZoomControls = false
        displayZoomControls = false
        // Follow the system font size, like the rest of the screen.
        textZoom = (fontScale * 100).toInt().coerceIn(50, 300)
    }
    CookieManager.getInstance().apply {
        // No other web view in the app needs cookies.
        setAcceptCookie(false)
        setAcceptThirdPartyCookies(view, false)
    }
}

/**
 * Serves `mailcrane-cid:` images, blocks every other request unless remote content is allowed
 * (then only `https:`), and never navigates: tapped links go to [onOpenLink].
 */
open class SafeWebViewClient(
    private val inlinePart: (contentId: String) -> InlinePart?,
    private val onOpenLink: (Uri) -> Unit,
) : WebViewClient() {
    /** Read on the web view's background thread. */
    @Volatile var allowRemote: Boolean = false

    override fun shouldInterceptRequest(
        view: WebView,
        request: WebResourceRequest,
    ): WebResourceResponse? = intercept(request.url)

    /** The response for [url]: null lets the web view load it. Public for tests. */
    fun intercept(url: Uri): WebResourceResponse? =
        when (url.scheme?.lowercase()) {
            CID_SCHEME -> serveCid(url)
            // The page itself and inline `data:` images; neither touches the network.
            "data", "about" -> null
            "https" -> if (allowRemote) null else blocked()
            else -> blocked()
        }

    private fun serveCid(url: Uri): WebResourceResponse {
        if (url.host != CID_HOST) return blocked()
        val contentId = url.pathSegments.singleOrNull() ?: return blocked()
        val part =
            try {
                inlinePart(contentId)
            } catch (_: RuntimeException) {
                null
            } ?: return blocked(404, "Not Found")
        return WebResourceResponse(part.mimeType, null, 200, "OK", emptyMap(), ByteArrayInputStream(part.bytes))
    }

    override fun shouldOverrideUrlLoading(
        view: WebView,
        request: WebResourceRequest,
    ): Boolean {
        // Only a tap on a link opens anything; a page can't send the user elsewhere by itself.
        if (request.hasGesture()) openExternally(request.url)
        return true
    }

    private fun openExternally(url: Uri) {
        if (url.scheme?.lowercase() in EXTERNAL_LINK_SCHEMES) onOpenLink(url)
    }
}

private fun blocked(
    status: Int = 403,
    reason: String = "Forbidden",
) = WebResourceResponse("text/plain", "utf-8", status, reason, emptyMap(), ByteArrayInputStream(ByteArray(0)))
