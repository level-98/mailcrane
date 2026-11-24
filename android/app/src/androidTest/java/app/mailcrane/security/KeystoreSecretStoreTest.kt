package app.mailcrane.security

import android.content.Context
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.mailcrane.core.SecretStoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.security.KeyStore
import java.util.UUID
import javax.crypto.SecretKey

@RunWith(AndroidJUnit4::class)
class KeystoreSecretStoreTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val alias = "mailcrane_secrets_test"
    private lateinit var file: File
    private lateinit var scope: CoroutineScope
    private lateinit var store: KeystoreSecretStore

    @Before
    fun setUp() {
        deleteKey()
        val name = "secrets_test_${UUID.randomUUID()}"
        file = context.preferencesDataStoreFile(name)
        scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
        val dataStore = PreferenceDataStoreFactory.create(scope = scope, produceFile = { file })
        store = KeystoreSecretStore(dataStore, alias)
    }

    @After
    fun tearDown() {
        scope.cancel()
        file.delete()
        deleteKey()
    }

    @Test
    fun putGetDeleteRoundTrips() {
        val key = "account/1/password"
        store.put(key, "pw-example".toByteArray())
        assertArrayEquals("pw-example".toByteArray(), store.get(key))

        store.delete(key)
        assertNull(store.get(key))
        // Deleting a missing key is not an error.
        store.delete(key)
    }

    @Test
    fun putReplacesTheOldValue() {
        val key = "account/1/password"
        store.put(key, "old-example".toByteArray())
        store.put(key, "new-example".toByteArray())
        assertArrayEquals("new-example".toByteArray(), store.get(key))
    }

    @Test
    fun putWipesTheCallersBuffer() {
        val value = "pw-example".toByteArray()
        store.put("account/1/password", value)
        assertTrue(value.all { it == 0.toByte() })
    }

    @Test
    fun theFileNeverHoldsThePlaintext() {
        store.put("account/1/password", "plaintext-marker-example".toByteArray())
        val onDisk = file.readBytes().toString(Charsets.ISO_8859_1)
        assertFalse(onDisk.contains("plaintext-marker-example"))
    }

    @Test
    fun theKeyIsInAndroidKeyStoreAndNotExportable() {
        store.put("account/1/password", "pw-example".toByteArray())
        val key = keyStore().getKey(alias, null) as SecretKey
        // AndroidKeyStore keys never hand out their raw bytes.
        assertNull(key.encoded)
    }

    @Test
    fun aLostKeyIsReportedAsInvalidatedNotACrash() {
        val key = "account/1/password"
        store.put(key, "pw-example".toByteArray())
        // What a restore onto a new device looks like: the file is there but
        // the key isn't.
        deleteKey()
        try {
            store.get(key)
            fail("expected Invalidated")
        } catch (e: SecretStoreException.Invalidated) {
            // expected
        }
        // Entering the password again works.
        store.put(key, "pw-again-example".toByteArray())
        assertArrayEquals("pw-again-example".toByteArray(), store.get(key))
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun deleteKey() {
        val ks = keyStore()
        if (ks.containsAlias(alias)) ks.deleteEntry(alias)
    }
}
