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

    /**
     * Looks for server settings for [email] (M1-15), most trusted first, within 20 seconds. The
     * address is never sent anywhere.
     *
     * @throws DiscoverException when nothing was found; the app then offers manual setup.
     */
    suspend fun discoverSettings(email: String): List<DiscoveredSettings> = throw DiscoverException.NotFound()

    /** Whether [email] is a gmail.com or googlemail.com address, so setup offers Sign in with Google (M3-02). */
    fun isGmailAddress(email: String): Boolean = false

    /** Google's OAuth settings for [app.mailcrane.oauth.OAuthSignIn]. @throws OAuthFailure */
    fun googleOAuthProvider(): OAuthProviderConfig = throw OAuthFailure.InvalidProvider()

    /**
     * Adds a Google account after Sign in with Google (M3-02): [signInId] is what the redirect
     * returned. Logs in to Gmail with the token, then saves. @throws AddAccountException
     */
    suspend fun addAccountGoogle(
        signInId: String,
        email: String,
        displayName: String,
    ): Long = throw AddAccountException.NotReady()

    /**
     * Adds a JMAP account (M3-04): finds the session for [JmapAccountSetup.server] (a domain or the
     * session URL), logs in to check, then saves. Same errors as [addAccountManual].
     */
    suspend fun addAccountJmap(
        setup: JmapAccountSetup,
        password: String,
    ): Long = throw AddAccountException.NotReady()

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
    private val dns: PlatformDns? = AndroidDns(),
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

    override suspend fun addAccountJmap(
        setup: JmapAccountSetup,
        password: String,
    ): Long = app.mailcrane.core.addAccountJmap(dbPath, setup, password)

    override suspend fun discoverSettings(email: String): List<DiscoveredSettings> =
        app.mailcrane.core.discoverSettings(email, dns)

    override fun isGmailAddress(email: String): Boolean = app.mailcrane.core.isGmailAddress(email)

    override fun googleOAuthProvider(): OAuthProviderConfig = app.mailcrane.core.googleOauthProvider()

    override suspend fun addAccountGoogle(
        signInId: String,
        email: String,
        displayName: String,
    ): Long = app.mailcrane.core.addAccountGoogle(dbPath, signInId, email, displayName)

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
