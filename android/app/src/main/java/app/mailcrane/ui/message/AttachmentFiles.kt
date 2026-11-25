package app.mailcrane.ui.message

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/** Where opened attachments are written; cleared of files older than a day at start-up (M2-04). */
fun attachmentCacheDir(context: Context): File = File(context.cacheDir, "attachments")

/** How long a downloaded attachment is kept in the cache. */
private const val CACHE_MAX_AGE_MS = 24L * 60 * 60 * 1000

/** Deletes cached attachments older than a day. Call off the main thread. */
fun clearOldAttachments(
    context: Context,
    now: Long = System.currentTimeMillis(),
) {
    attachmentCacheDir(context).listFiles()?.forEach { f ->
        if (now - f.lastModified() > CACHE_MAX_AGE_MS) f.delete()
    }
}

/** File types that can run code or scripts when opened; opening them asks first. */
private val RISKY_EXTENSIONS =
    setOf(
        "apk",
        "aab",
        "xapk",
        "exe",
        "msi",
        "bat",
        "cmd",
        "com",
        "scr",
        "ps1",
        "vbs",
        "js",
        "jse",
        "jar",
        "sh",
        "html",
        "htm",
        "xhtml",
        "svg",
        "hta",
        "lnk",
        "dll",
        "app",
        "dmg",
        "pkg",
        "deb",
        "rpm",
        "py",
        "wsf",
        "reg",
    )

/** Whether opening [name] (or a file of [mimeType]) should be confirmed first. */
fun isRiskyAttachment(
    name: String,
    mimeType: String,
): Boolean {
    val ext = name.substringAfterLast('.', "").lowercase()
    val type = mimeType.lowercase()
    return ext in RISKY_EXTENSIONS ||
        type == "application/vnd.android.package-archive" ||
        type == "text/html" ||
        type == "image/svg+xml" ||
        type.contains("javascript") ||
        type == "application/x-msdownload"
}

/** Opens [file] in another app through the file provider. Returns false if no app can open it. */
fun openAttachment(
    context: Context,
    file: File,
    mimeType: String,
): Boolean {
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
    val intent =
        Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, mimeType.ifBlank { "application/octet-stream" })
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    return try {
        context.startActivity(Intent.createChooser(intent, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        true
    } catch (_: ActivityNotFoundException) {
        false
    }
}

/** Copies [file] to a document the user picked. */
suspend fun copyToDocument(
    context: Context,
    file: File,
    target: Uri,
): Boolean =
    withContext(Dispatchers.IO) {
        try {
            context.contentResolver.openOutputStream(target)?.use { out ->
                file.inputStream().use { it.copyTo(out) }
            } !=
                null
        } catch (_: Exception) {
            false
        }
    }
