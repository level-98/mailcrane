package app.mailcrane.ui.compose

import java.io.Serializable

/**
 * Open the compose screen on a saved draft: [draftId] for one saved on this device, or [messageId] for a
 * message in the Drafts folder. Serializable so it survives rotation and process death.
 */
data class DraftRequest(
    val draftId: Long? = null,
    val messageId: Long? = null,
) : Serializable
