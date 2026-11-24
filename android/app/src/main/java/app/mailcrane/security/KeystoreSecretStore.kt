package app.mailcrane.security

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import app.mailcrane.core.PlatformSecretStore
import app.mailcrane.core.SecretStoreException
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.KeyStoreException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The file the encrypted secrets live in. Excluded from backup in `res/xml`. */
private val Context.secretsDataStore: DataStore<Preferences> by preferencesDataStore(
    name = KeystoreSecretStore.FILE_NAME,
)

/**
 * Keeps credentials encrypted with an AES-256-GCM key held in `AndroidKeyStore`.
 *
 * The key never leaves the keystore (it can't be exported). Each value is
 * encrypted with a fresh IV, and the IV and ciphertext are kept in an
 * app-private DataStore file. The secret's name is bound in as associated
 * data, so one entry can't be swapped for another.
 *
 * Every failure is reported as a [SecretStoreException], never a crash, and
 * no key, value or platform message is logged.
 *
 * The core calls this from its own threads, never the main thread, so the
 * blocking DataStore calls here are fine.
 */
class KeystoreSecretStore internal constructor(
    private val dataStore: DataStore<Preferences>,
    private val keyAlias: String,
) : PlatformSecretStore {
    constructor(context: Context) : this(context.applicationContext.secretsDataStore, KEY_ALIAS)

    override fun put(
        key: String,
        value: ByteArray,
    ): Unit =
        guard {
            try {
                val cipher = Cipher.getInstance(TRANSFORMATION)
                cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey())
                cipher.updateAAD(key.toByteArray(Charsets.UTF_8))
                val ciphertext = cipher.doFinal(value)
                val stored = Base64.encodeToString(cipher.iv + ciphertext, Base64.NO_WRAP)
                runBlocking { dataStore.edit { it[stringPreferencesKey(key)] = stored } }
                Unit
            } finally {
                value.fill(0)
            }
        }

    override fun get(key: String): ByteArray? =
        guard {
            val stored =
                runBlocking { dataStore.data.first()[stringPreferencesKey(key)] }
                    ?: return@guard null
            // A value with no key means the file came from somewhere the key
            // doesn't exist (a restore, or the key was wiped).
            val secretKey = loadKey() ?: throw SecretStoreException.Invalidated()
            val bytes =
                try {
                    Base64.decode(stored, Base64.NO_WRAP)
                } catch (e: IllegalArgumentException) {
                    throw SecretStoreException.Invalidated()
                }
            if (bytes.size <= IV_BYTES) throw SecretStoreException.Invalidated()
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(
                Cipher.DECRYPT_MODE,
                secretKey,
                GCMParameterSpec(TAG_BITS, bytes, 0, IV_BYTES),
            )
            cipher.updateAAD(key.toByteArray(Charsets.UTF_8))
            cipher.doFinal(bytes, IV_BYTES, bytes.size - IV_BYTES)
        }

    override fun delete(key: String): Unit =
        guard {
            runBlocking { dataStore.edit { it.remove(stringPreferencesKey(key)) } }
            Unit
        }

    private fun keyStore(): KeyStore = KeyStore.getInstance(ANDROID_KEY_STORE).apply { load(null) }

    private fun loadKey(): SecretKey? = keyStore().getKey(keyAlias, null) as? SecretKey

    private fun getOrCreateKey(): SecretKey {
        loadKey()?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEY_STORE)
        generator.init(
            KeyGenParameterSpec
                .Builder(keyAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(KEY_BITS)
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return generator.generateKey()
    }

    /**
     * Runs [block] and turns every failure into a [SecretStoreException].
     * Only the exception's class name is kept, never its message.
     */
    private inline fun <T> guard(block: () -> T): T =
        try {
            block()
        } catch (e: SecretStoreException) {
            throw e
        } catch (e: KeyPermanentlyInvalidatedException) {
            throw SecretStoreException.Invalidated()
        } catch (e: AEADBadTagException) {
            throw SecretStoreException.Invalidated()
        } catch (e: UnrecoverableKeyException) {
            throw SecretStoreException.Invalidated()
        } catch (e: KeyStoreException) {
            throw SecretStoreException.Unavailable()
        } catch (e: GeneralSecurityException) {
            throw SecretStoreException.Other(e.javaClass.simpleName)
        } catch (e: Exception) {
            throw SecretStoreException.Other(e.javaClass.simpleName)
        }

    companion object {
        /** The DataStore file name; must match the backup and data extraction rules in `res/xml`. */
        const val FILE_NAME = "mailcrane_secrets"
        const val KEY_ALIAS = "mailcrane_secrets_v1"
        private const val ANDROID_KEY_STORE = "AndroidKeyStore"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val KEY_BITS = 256
        private const val TAG_BITS = 128
        private const val IV_BYTES = 12
    }
}
