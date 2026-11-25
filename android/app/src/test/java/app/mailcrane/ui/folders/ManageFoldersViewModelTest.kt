package app.mailcrane.ui.folders

import app.mailcrane.R
import app.mailcrane.core.FolderAdminCore
import app.mailcrane.core.FolderFailure
import app.mailcrane.core.FolderNameProblem
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MoveTargets
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private const val ACCOUNT = 1L

private val INBOX = FolderSummary(1, "INBOX", FolderRole.INBOX, 0u, 0u, canChange = false)
private val WORK = FolderSummary(2, "Work", null, 0u, 0u, canChange = true)

private class FakeFolderAdmin : FolderAdminCore {
    var folders = listOf(INBOX, WORK)
    var manageable = true
    var failure: FolderFailure? = null
    val calls = mutableListOf<String>()

    override suspend fun listFolders(accountId: Long) = folders

    override suspend fun canManageFolders(accountId: Long) = manageable

    override suspend fun createFolder(
        accountId: Long,
        parentId: Long?,
        name: String,
    ): Long {
        failure?.let { throw it }
        calls += "create($parentId,$name)"
        folders = folders + FolderSummary(3, name, null, if (parentId == null) 0u else 1u, 0u, canChange = true)
        return 3
    }

    override suspend fun renameFolder(
        folderId: Long,
        name: String,
    ) {
        failure?.let { throw it }
        calls += "rename($folderId,$name)"
    }

    override suspend fun moveFolder(
        folderId: Long,
        parentId: Long?,
    ) {
        failure?.let { throw it }
        calls += "move($folderId,$parentId)"
    }

    override suspend fun deleteFolder(folderId: Long) {
        failure?.let { throw it }
        calls += "delete($folderId)"
        folders = folders.filter { it.id != folderId }
    }

    override suspend fun moveTargets(folderId: Long) = MoveTargets(toTop = false, folders = listOf(INBOX))
}

@OptIn(ExperimentalCoroutinesApi::class)
class ManageFoldersViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun createsAFolderUnderAParentAndCloses() {
        val core = FakeFolderAdmin()
        val vm = ManageFoldersViewModel(core, ACCOUNT)
        assertTrue(vm.state.value.canManage)
        vm.startCreate(WORK)
        assertEquals(FolderDialog.Create(WORK), vm.state.value.dialog)
        vm.confirmCreate("Receipts")
        assertEquals(listOf("create(2,Receipts)"), core.calls)
        assertNull(vm.state.value.dialog)
        assertEquals(3, vm.state.value.folders.size)
    }

    @Test
    fun aRefusedNameKeepsTheDialogOpenWithTheReason() {
        val core = FakeFolderAdmin().apply { failure = FolderFailure.InvalidName(FolderNameProblem.Separator("/")) }
        val vm = ManageFoldersViewModel(core, ACCOUNT)
        vm.startRename(WORK)
        vm.confirmRename("a/b")
        assertEquals(FolderDialog.Rename(WORK), vm.state.value.dialog)
        assertEquals(FolderMessage(R.string.folders_error_separator, "/"), vm.state.value.error)
        // Dismissing clears it.
        vm.dismiss()
        assertNull(vm.state.value.dialog)
        assertNull(vm.state.value.error)
    }

    @Test
    fun moveLoadsTheTargetsThenMoves() {
        val core = FakeFolderAdmin()
        val vm = ManageFoldersViewModel(core, ACCOUNT)
        vm.startMove(WORK)
        val dialog = vm.state.value.dialog as FolderDialog.Move
        assertEquals(listOf(INBOX), dialog.targets?.folders)
        vm.confirmMove(INBOX.id)
        assertEquals(listOf("move(2,1)"), core.calls)
        assertNull(vm.state.value.dialog)
    }

    @Test
    fun deleteAsksFirstAndReportsSubfolders() {
        val core = FakeFolderAdmin().apply { failure = FolderFailure.HasSubfolders() }
        val vm = ManageFoldersViewModel(core, ACCOUNT)
        vm.startDelete(WORK)
        assertEquals(FolderDialog.Delete(WORK), vm.state.value.dialog)
        assertTrue(core.calls.isEmpty())
        vm.confirmDelete()
        assertEquals(FolderMessage(R.string.folders_error_has_subfolders), vm.state.value.error)
        core.failure = null
        vm.confirmDelete()
        assertEquals(listOf("delete(2)"), core.calls)
        assertEquals(listOf(INBOX), vm.state.value.folders)
    }

    @Test
    fun offlineAndUnsupportedAccountsAreExplained() {
        val core = FakeFolderAdmin().apply { failure = FolderFailure.Sync(app.mailcrane.core.SyncErrorKind.NETWORK) }
        val vm = ManageFoldersViewModel(core, ACCOUNT)
        vm.startCreate(null)
        vm.confirmCreate("X")
        assertEquals(FolderMessage(R.string.folders_error_offline), vm.state.value.error)

        val jmap = ManageFoldersViewModel(FakeFolderAdmin().apply { manageable = false }, ACCOUNT)
        assertEquals(false, jmap.state.value.canManage)
    }
}
