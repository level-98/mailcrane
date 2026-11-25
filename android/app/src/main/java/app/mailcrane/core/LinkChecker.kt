package app.mailcrane.core

/** Asks the core where a link really goes (M5-07). A fun interface so tests can fake it. */
fun interface LinkChecker {
    /** The destination and warnings for [url], given the [text] the reader sees for it, if known. */
    fun check(
        url: String,
        text: String?,
    ): LinkCheck
}

/** [LinkChecker] backed by the core's `check_link`. */
object CoreLinkChecker : LinkChecker {
    override fun check(
        url: String,
        text: String?,
    ): LinkCheck = checkLink(url, text)
}
