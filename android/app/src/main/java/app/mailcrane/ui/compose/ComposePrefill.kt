package app.mailcrane.ui.compose

import java.io.Serializable

/**
 * Open the compose screen with these fields filled in, for the user to check and send (for example an
 * unsubscribe email, M4-10). Never sent without the user. Serializable so it survives rotation.
 */
data class ComposePrefill(
    val to: String,
    val subject: String = "",
    val body: String = "",
) : Serializable
