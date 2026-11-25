package app.mailcrane.ui.compose

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import app.mailcrane.core.ComposeAttachment
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Largest single file read into a message (the core refuses more than 35 MB in total). */
private const val MAX_FILE_BYTES = 35L * 1024 * 1024

/**
 * Reads picked files (M2-04): name and type from the provider, contents into memory. Files that can't
 * be read, or are too large, are left out.
 */
suspend fun readAttachments(
    context: Context,
    uris: List<Uri>,
): List<ComposeAttachment> =
    withContext(Dispatchers.IO) {
        uris.mapNotNull { uri ->
            try {
                val resolver = context.contentResolver
                var name = "attachment"
                var size = -1L
                resolver
                    .query(
                        uri,
                        arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE),
                        null,
                        null,
                        null,
                    )?.use {
                        if (it.moveToFirst()) {
                            name = it.getString(0) ?: name
                            if (!it.isNull(1)) size = it.getLong(1)
                        }
                    }
                if (size > MAX_FILE_BYTES) return@mapNotNull null
                val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: return@mapNotNull null
                if (bytes.size > MAX_FILE_BYTES) return@mapNotNull null
                ComposeAttachment(name, resolver.getType(uri) ?: "application/octet-stream", bytes)
            } catch (_: Exception) {
                null
            }
        }
    }
