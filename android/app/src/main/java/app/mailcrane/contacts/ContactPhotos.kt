package app.mailcrane.contacts

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.ContactsContract
import android.util.LruCache
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.content.ContextCompat
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.IOException
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Sender photos from the device's contacts (M4-24), used only after the user turned on "Use photos from
 * your contacts" and allowed reading contacts. Photos are looked up by email address, off the main thread,
 * and kept in memory only: nothing is stored or sent anywhere.
 */
interface ContactPhotos {
    /** The contact photo for [address], or null (none, no permission, or the setting is off). */
    suspend fun photo(address: String): ImageBitmap?
}

/** No photos: the default, and while the setting is off. */
object NoContactPhotos : ContactPhotos {
    override suspend fun photo(address: String): ImageBitmap? = null
}

/** The photos the message list shows; [NoContactPhotos] unless the user opted in. */
val LocalContactPhotos = staticCompositionLocalOf<ContactPhotos> { NoContactPhotos }

/** Reads thumbnails through `ContactsContract` while `READ_CONTACTS` is granted. */
@Singleton
class AndroidContactPhotos
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
    ) : ContactPhotos {
        /** Looked-up addresses, including misses, so scrolling never asks twice. */
        private val cache = LruCache<String, Found>(CACHE_SIZE)

        private val allowed: Boolean
            get() =
                ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CONTACTS) ==
                    PackageManager.PERMISSION_GRANTED

        override suspend fun photo(address: String): ImageBitmap? {
            val key = address.trim().lowercase()
            if (key.isEmpty() || '@' !in key || !allowed) return null
            cache.get(key)?.let { return it.bitmap }
            val found = withContext(Dispatchers.IO) { Found(load(key)) }
            cache.put(key, found)
            return found.bitmap
        }

        private fun load(address: String): ImageBitmap? {
            val uri =
                Uri.withAppendedPath(
                    ContactsContract.CommonDataKinds.Email.CONTENT_LOOKUP_URI,
                    Uri.encode(address),
                )
            val projection = arrayOf(ContactsContract.CommonDataKinds.Email.PHOTO_THUMBNAIL_URI)
            return try {
                val thumbnail =
                    context.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
                        if (cursor.moveToFirst()) cursor.getString(0) else null
                    } ?: return null
                context.contentResolver.openInputStream(Uri.parse(thumbnail))?.use { stream ->
                    BitmapFactory.decodeStream(stream)?.asImageBitmap()
                }
            } catch (_: SecurityException) {
                // The permission was taken back meanwhile.
                null
            } catch (_: IOException) {
                null
            } catch (_: IllegalArgumentException) {
                null
            }
        }

        private class Found(
            val bitmap: ImageBitmap?,
        )

        private companion object {
            const val CACHE_SIZE = 200
        }
    }
