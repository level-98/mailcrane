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
}
