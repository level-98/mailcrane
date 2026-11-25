package app.mailcrane.core

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Local rules kept by the core (M4-09). An interface so view models can be tested with a fake. */
interface RulesCore {
    /** Every rule, in the order they run. @throws RulesFailure */
    suspend fun listRules(): List<RuleInfo>

    /** Saves a rule (a new one when [id] is null) and returns its id. @throws RulesFailure */
    suspend fun saveRule(
        id: Long?,
        rule: RuleInput,
    ): Long

    /** @throws RulesFailure */
    suspend fun deleteRule(id: Long)

    /** @throws RulesFailure */
    suspend fun setRuleEnabled(
        id: Long,
        enabled: Boolean,
    )

    /** Moves a rule to [index] (0 runs first). @throws RulesFailure */
    suspend fun moveRule(
        id: Long,
        index: Int,
    )
}

/** Calls the core through the generated UniFFI bindings. [dbPath] is the app's database file. */
class UniffiRulesCore(
    private val dbPath: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : RulesCore {
    override suspend fun listRules(): List<RuleInfo> = withContext(io) { listRules(dbPath) }

    override suspend fun saveRule(
        id: Long?,
        rule: RuleInput,
    ): Long = withContext(io) { saveRule(dbPath, id, rule) }

    override suspend fun deleteRule(id: Long) = withContext(io) { deleteRule(dbPath, id) }

    override suspend fun setRuleEnabled(
        id: Long,
        enabled: Boolean,
    ) = withContext(io) { setRuleEnabled(dbPath, id, enabled) }

    override suspend fun moveRule(
        id: Long,
        index: Int,
    ) = withContext(io) { moveRule(dbPath, id, index.coerceAtLeast(0).toUInt()) }
}
