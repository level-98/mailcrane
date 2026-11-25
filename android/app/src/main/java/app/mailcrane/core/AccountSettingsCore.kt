package app.mailcrane.core

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Per-account settings kept by the core. An interface so view models can be tested with a fake. */
interface AccountSettingsCore {
    /** @throws SettingsFailure when the account doesn't exist or the database fails. */
    suspend fun getSignature(accountId: Long): AccountSignature

    /** @throws SettingsFailure, for example [SettingsFailure.TooLong]. */
    suspend fun setSignature(
        accountId: Long,
        signature: AccountSignature,
    )

    /** "Save sent messages to the Sent folder". @throws SettingsFailure */
    suspend fun getSaveSent(accountId: Long): SaveSentSetting

    /** @throws SettingsFailure */
    suspend fun setSaveSent(
        accountId: Long,
        setting: SaveSentSetting,
    )

    /** The marker colour the user chose (0–7), or null when automatic (M4-20). @throws SettingsFailure */
    suspend fun getAccountColor(accountId: Long): Int?

    /** Chooses the marker colour (0–7), or null for automatic. @throws SettingsFailure */
    suspend fun setAccountColor(
        accountId: Long,
        color: Int?,
    )

    /** The colour the account gets when none is chosen. */
    fun automaticAccountColor(accountId: Long): Int

    /** The provider's authentication server name the user set (M5-08), or null. @throws SettingsFailure */
    suspend fun getProviderAuthservId(accountId: Long): String?

    /**
     * Saves the provider's authentication server name as typed; empty clears it. Returns the value
     * saved, as the core cleaned it. @throws SettingsFailure, for example [SettingsFailure.InvalidAuthservId].
     */
    suspend fun setProviderAuthservId(
        accountId: Long,
        value: String,
    ): String?
}

/** Calls the core through the generated UniFFI bindings. [dbPath] is the app's database file. */
class UniffiAccountSettingsCore(
    private val dbPath: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : AccountSettingsCore {
    override suspend fun getSignature(accountId: Long): AccountSignature =
        withContext(io) { getSignature(dbPath, accountId) }

    override suspend fun setSignature(
        accountId: Long,
        signature: AccountSignature,
    ) = withContext(io) { setSignature(dbPath, accountId, signature) }

    override suspend fun getSaveSent(accountId: Long): SaveSentSetting =
        withContext(io) { getSaveSent(dbPath, accountId) }

    override suspend fun setSaveSent(
        accountId: Long,
        setting: SaveSentSetting,
    ) = withContext(io) { setSaveSent(dbPath, accountId, setting) }

    override suspend fun getAccountColor(accountId: Long): Int? =
        withContext(io) { getAccountColor(dbPath, accountId)?.toInt() }

    override suspend fun setAccountColor(
        accountId: Long,
        color: Int?,
    ) = withContext(io) {
        // A negative index would wrap to a huge unsigned one, which the core refuses.
        setAccountColor(dbPath, accountId, color?.toUInt())
    }

    override fun automaticAccountColor(accountId: Long): Int = accountColor(accountId).toInt()

    override suspend fun getProviderAuthservId(accountId: Long): String? =
        withContext(io) { getProviderAuthservId(dbPath, accountId) }

    override suspend fun setProviderAuthservId(
        accountId: Long,
        value: String,
    ): String? = withContext(io) { setProviderAuthservId(dbPath, accountId, value) }
}
