package app.mailcrane.ui.account

import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.JmapAccountSetup
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import app.mailcrane.core.OAuthProviderConfig

/** A core for the account setup tests: adding an account returns [result]'s id or throws. */
internal class FakeSetupCore(
    private val result: () -> Long,
) : MailcraneCore {
    var lastConfig: ManualAccountConfig? = null
    var lastJmap: JmapAccountSetup? = null
    var lastGoogle: Triple<String, String, String>? = null

    override fun isGmailAddress(email: String) = email.endsWith("@gmail.com")

    override fun googleOAuthProvider() =
        OAuthProviderConfig(
            id = "google",
            authorizationEndpoint = "https://accounts.example.org/auth",
            tokenEndpoint = "https://oauth2.example.org/token",
            clientId = "client",
            scopes = listOf("https://mail.google.com/"),
            redirectUri = "app.mailcrane:/oauth2redirect",
            extraAuthParams = emptyList(),
        )

    override suspend fun addAccountGoogle(
        signInId: String,
        email: String,
        displayName: String,
    ): Long {
        lastGoogle = Triple(signInId, email, displayName)
        return result()
    }

    override fun hello() = ""

    override fun defaultPort(
        protocol: MailServer,
        security: ConnectionSecurity,
    ) = when (protocol to security) {
        MailServer.IMAP to ConnectionSecurity.TLS -> 993
        MailServer.IMAP to ConnectionSecurity.START_TLS -> 143
        MailServer.SMTP to ConnectionSecurity.TLS -> 465
        else -> 587
    }

    override suspend fun addAccountManual(
        config: ManualAccountConfig,
        password: String,
    ): Long {
        lastConfig = config
        return result()
    }

    override suspend fun addAccountJmap(
        setup: JmapAccountSetup,
        password: String,
    ): Long {
        lastJmap = setup
        return result()
    }
}
