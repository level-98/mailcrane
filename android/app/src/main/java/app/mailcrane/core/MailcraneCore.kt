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

    /** The saved accounts, oldest first (M4-01). */
    suspend fun listAccounts(): List<AccountSummary> = emptyList()

    /** Removes an account, its password and all its mail from this device. @throws RemoveAccountFailure */
    suspend fun removeAccount(accountId: Long) {}
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

    override suspend fun listAccounts(): List<AccountSummary> =
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            try {
                app.mailcrane.core.listAccounts(dbPath)
            } catch (_: SyncFailure) {
                emptyList()
            }
        }

    override suspend fun removeAccount(accountId: Long) =
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            app.mailcrane.core.removeAccount(dbPath, accountId)
        }
}
