package app.mailcrane.contacts

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.provider.ContactsContract
import androidx.core.content.ContextCompat
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import javax.inject.Inject

/** A recipient to suggest while typing (M2-08). */
data class RecipientSuggestion(
    val name: String,
    val email: String,
    /** From the device's contacts rather than past correspondence. */
    val fromDevice: Boolean = false,
)

/**
 * The device's contacts, read only after the user allowed it. Nothing read here is given to the core or
 * stored: suggestions are merged in memory while typing.
 */
interface DeviceContacts {
    /** Whether the user allowed reading contacts. */
    val available: Boolean

    /** Up to [limit] contacts with an email address matching [prefix]; empty without permission. */
    suspend fun search(
        prefix: String,
        limit: Int,
    ): List<RecipientSuggestion>
}

/** No device contacts (tests, or before permission). */
object NoDeviceContacts : DeviceContacts {
    override val available: Boolean = false

    override suspend fun search(
        prefix: String,
        limit: Int,
    ): List<RecipientSuggestion> = emptyList()
}

/** Reads `ContactsContract` email rows when `READ_CONTACTS` is granted. */
class AndroidDeviceContacts
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
    ) : DeviceContacts {
        override val available: Boolean
            get() =
                ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CONTACTS) ==
                    PackageManager.PERMISSION_GRANTED

        override suspend fun search(
            prefix: String,
            limit: Int,
        ): List<RecipientSuggestion> {
            val query = prefix.trim()
            if (!available || query.isEmpty() || limit <= 0) return emptyList()
            return withContext(Dispatchers.IO) {
                val uri =
                    ContactsContract.CommonDataKinds.Email.CONTENT_FILTER_URI
                        .buildUpon()
                        .appendPath(query)
                        .build()
                val projection =
                    arrayOf(
                        ContactsContract.Contacts.DISPLAY_NAME_PRIMARY,
                        ContactsContract.CommonDataKinds.Email.ADDRESS,
                    )
                val out = mutableListOf<RecipientSuggestion>()
                try {
                    context.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
                        while (cursor.moveToNext() && out.size < limit) {
                            val email = cursor.getString(1)?.trim().orEmpty()
                            if ('@' !in email) continue
                            out += RecipientSuggestion(cursor.getString(0).orEmpty(), email, fromDevice = true)
                        }
                    }
                } catch (_: SecurityException) {
                    // Permission taken back meanwhile: no device suggestions.
                }
                out
            }
        }
    }

/**
 * Past correspondents first (they are ranked), then device contacts not already listed, without
 * duplicate addresses (compared without case), at most [limit].
 */
fun mergeSuggestions(
    correspondents: List<RecipientSuggestion>,
    device: List<RecipientSuggestion>,
    limit: Int,
): List<RecipientSuggestion> {
    val seen = HashSet<String>()
    val out = mutableListOf<RecipientSuggestion>()
    for (s in correspondents + device) {
        if (out.size >= limit) break
        val key = s.email.trim().lowercase()
        if (key.isEmpty() || !seen.add(key)) continue
        // A device contact's name is often better than the one in an email header.
        val name =
            if (s.name.isBlank() && !s.fromDevice) {
                device.firstOrNull { it.email.trim().lowercase() == key }?.name ?: s.name
            } else {
                s.name
            }
        out += s.copy(name = name)
    }
    return out
}
