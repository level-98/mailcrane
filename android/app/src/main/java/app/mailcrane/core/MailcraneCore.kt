package app.mailcrane.core

/** What the apps can ask the Rust core. An interface so view models can be tested with a fake. */
interface MailcraneCore {
    fun hello(): String
}

/** Calls the core through the generated UniFFI bindings (the top-level `hello()` in this package). */
class UniffiMailcraneCore : MailcraneCore {
    override fun hello(): String = app.mailcrane.core.hello()
}
