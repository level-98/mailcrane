package app.mailcrane.ui.compose

import app.mailcrane.core.ReplyKind
import java.io.Serializable

/**
 * Open the compose screen as a reply, reply-all or forward of message [messageId]. [quoteHeader] is the
 * localised line above the quoted text ("On 24 Nov 2026, Ana wrote:") or above a forward's headers.
 * Serializable so the request survives rotation and process death.
 */
data class ReplyRequest(
    val messageId: Long,
    val kind: ReplyKind,
    val quoteHeader: String,
) : Serializable
