package app.mailcrane.core

/**
 * Creating, renaming, moving and deleting folders (M4-22). The core decides everything; the changes
 * are made on the server first. An interface so the screen can be tested with a fake.
 */
interface FolderAdminCore {
    /** The account's folders in display order. @throws SyncFailure */
    suspend fun listFolders(accountId: Long): List<FolderSummary>

    /** Whether folders can be managed in this account (not JMAP accounts yet). */
    suspend fun canManageFolders(accountId: Long): Boolean

    /** Creates folder [name] under [parentId], or at the top. Returns its id. @throws FolderFailure */
    suspend fun createFolder(
        accountId: Long,
        parentId: Long?,
        name: String,
    ): Long

    /** @throws FolderFailure */
    suspend fun renameFolder(
        folderId: Long,
        name: String,
    )

    /** Moves the folder under [parentId], or to the top. @throws FolderFailure */
    suspend fun moveFolder(
        folderId: Long,
        parentId: Long?,
    )

    /** Deletes the folder and its messages. Ask the user first. @throws FolderFailure */
    suspend fun deleteFolder(folderId: Long)

    /** Where the folder can move to. @throws SyncFailure */
    suspend fun moveTargets(folderId: Long): MoveTargets
}
