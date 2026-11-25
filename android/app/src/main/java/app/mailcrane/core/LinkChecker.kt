package app.mailcrane.core

/** Asks the core where a link really goes (M5-07). A fun interface so tests can fake it. */
fun interface LinkChecker {
    /**
     * The destination and warnings for [target] (the link about to open, already cleaned). The core
     * finds the text the reader saw by matching [tapped] (the URL the web view reported) against
     * [links] from the HTML body; empty for plain-text messages.
     */
    fun check(
        tapped: String,
        target: String,
        links: List<LinkText>,
    ): LinkCheck
}

/** [LinkChecker] backed by the core's `link_text_for` and `check_link`. */
object CoreLinkChecker : LinkChecker {
    override fun check(
        tapped: String,
        target: String,
        links: List<LinkText>,
    ): LinkCheck = checkLink(target, if (links.isEmpty()) null else linkTextFor(tapped, links))
}
