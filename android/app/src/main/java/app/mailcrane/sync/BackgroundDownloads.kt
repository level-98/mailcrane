package app.mailcrane.sync

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.PowerManager
import android.os.StatFs
import androidx.core.content.ContextCompat
import app.mailcrane.core.BackgroundDownload
import app.mailcrane.core.DeviceConditions
import app.mailcrane.core.UniffiMailboxCore
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.BackgroundDownloadSetting
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import javax.inject.Inject
import javax.inject.Singleton

/** What the device is like right now, for deciding on background downloads (M4-25). */
data class DeviceState(
    /** Wi-Fi, Ethernet or another network that doesn't count data. */
    val unmetered: Boolean,
    val batterySaver: Boolean,
    /** Free bytes on the storage holding the database. */
    val freeBytes: Long,
)

/** The conditions the core decides on: the setting plus the device state. */
fun deviceConditions(
    setting: BackgroundDownloadSetting,
    state: DeviceState,
): DeviceConditions =
    DeviceConditions(
        setting =
            when (setting) {
                BackgroundDownloadSetting.WIFI_ONLY -> BackgroundDownload.WIFI_ONLY
                BackgroundDownloadSetting.ALWAYS -> BackgroundDownload.ALWAYS
                BackgroundDownloadSetting.NEVER -> BackgroundDownload.NEVER
            },
        unmetered = state.unmetered,
        batterySaver = state.batterySaver,
        freeBytes = state.freeBytes.coerceAtLeast(0L).toULong(),
    )

/**
 * Keeps the core told about the network, Battery Saver, free storage and the "Download messages in
 * the background" setting. The core downloads recent inbox messages after each sync when these
 * allow it (on Wi-Fi only by default, never in Battery Saver or with 500 MB or less free).
 */
@Singleton
class BackgroundDownloads
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
        private val settings: AppSettings,
        private val core: UniffiMailboxCore,
    ) {
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

        /** Call once from the Application. */
        fun start() {
            scope.launch {
                combine(settings.backgroundDownload, deviceStates(context), ::deviceConditions)
                    .distinctUntilChanged()
                    .collect { core.setDeviceConditions(it) }
            }
        }
    }

/** How often free storage is checked again while nothing else changes. */
private const val STORAGE_RECHECK_MS = 5 * 60 * 1000L

/** The device state now, and again whenever the network or Battery Saver changes. */
private fun deviceStates(context: Context): Flow<DeviceState> =
    callbackFlow {
        val connectivity = context.getSystemService(ConnectivityManager::class.java)
        val power = context.getSystemService(PowerManager::class.java)

        fun current(): DeviceState {
            val caps = connectivity?.let { c -> c.activeNetwork?.let { c.getNetworkCapabilities(it) } }
            val free =
                try {
                    StatFs(context.noBackupFilesDir.path).availableBytes
                } catch (e: IllegalArgumentException) {
                    0L
                }
            return DeviceState(
                unmetered = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) == true,
                batterySaver = power?.isPowerSaveMode == true,
                freeBytes = free,
            )
        }

        val network =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    trySend(current())
                }

                override fun onCapabilitiesChanged(
                    network: Network,
                    caps: NetworkCapabilities,
                ) {
                    trySend(current())
                }

                override fun onLost(network: Network) {
                    trySend(current())
                }
            }
        val batterySaver =
            object : BroadcastReceiver() {
                override fun onReceive(
                    context: Context,
                    intent: Intent,
                ) {
                    trySend(current())
                }
            }
        connectivity?.registerDefaultNetworkCallback(network)
        ContextCompat.registerReceiver(
            context,
            batterySaver,
            IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        val storage =
            launch {
                while (true) {
                    send(current())
                    delay(STORAGE_RECHECK_MS)
                }
            }
        awaitClose {
            storage.cancel()
            connectivity?.unregisterNetworkCallback(network)
            context.unregisterReceiver(batterySaver)
        }
    }.conflate()
