package app.mailcrane.ui.account

import app.mailcrane.core.AppPasswordGuide
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.JmapAccountSetup
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig

/** A core for the account setup tests: adding an account returns [result]'s id or throws. */
internal class FakeSetupCore(
    private val guides: Map<String, AppPasswordGuide> = emptyMap(),
    private val result: () -> Long,
) : MailcraneCore {
    override fun appPasswordGuide(email: String): AppPasswordGuide? = guides[email]

    var lastConfig: ManualAccountConfig? = null
    var lastJmap: JmapAccountSetup? = null

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
