package app.mailcrane.core

/** Removes tracking parameters from a link (`utm_*`, `fbclid` and others). A fun interface so tests can fake it. */
fun interface LinkCleaner {
    /** The link without tracking parameters; anything that isn't an http(s) link comes back unchanged. */
    fun clean(url: String): String
}

/** [LinkCleaner] backed by the core's `clean_url`. */
object CoreLinkCleaner : LinkCleaner {
    override fun clean(url: String): String = cleanUrl(url)
}
