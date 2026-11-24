package app.mailcrane.core

/** What the apps can ask the Rust core. An interface so view models can be tested with a fake. */
interface MailcraneCore {
    fun hello(): String

    /** The usual port for [protocol] with [security]: IMAP 993/143, SMTP 465/587. */
    fun defaultPort(
        protocol: MailServer,
        security: ConnectionSecurity,
    ): Int

    /**
     * Validates [config], logs in to both servers and only then saves the account, with the
     * password in secure storage. Returns the new account's id.
     *
     * @throws AddAccountException when anything fails; nothing is saved in that case.
     */
    suspend fun addAccountManual(
        config: ManualAccountConfig,
        password: String,
    ): Long
}

/**
 * Calls the core through the generated UniFFI bindings (the top-level functions in this package).
 * [dbPath] is the app's database file.
 */
class UniffiMailcraneCore(
    private val dbPath: String,
) : MailcraneCore {
    override fun hello(): String = app.mailcrane.core.hello()

    override fun defaultPort(
        protocol: MailServer,
        security: ConnectionSecurity,
    ): Int =
        app.mailcrane.core
            .defaultPort(protocol, security)
            .toInt()

    override suspend fun addAccountManual(
        config: ManualAccountConfig,
        password: String,
    ): Long = app.mailcrane.core.addAccountManual(dbPath, config, password)
}
