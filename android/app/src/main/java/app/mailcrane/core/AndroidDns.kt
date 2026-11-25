package app.mailcrane.core

import android.net.DnsResolver
import android.os.Build
import android.os.CancellationSignal
import androidx.annotation.RequiresApi
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * DNS lookups for automatic setup (M1-15) through the system resolver, so they go to the user's
 * own DNS server and nowhere else. Raw queries need Android 10 (API 29); on older versions every
 * lookup returns null and setup uses the provider's autoconfig file only.
 */
class AndroidDns : PlatformDns {
    override fun query(
        name: String,
        recordType: UShort,
    ): ByteArray? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            rawQuery(name, recordType.toInt())
        } else {
            null
        }

    @RequiresApi(Build.VERSION_CODES.Q)
    private fun rawQuery(
        name: String,
        type: Int,
    ): ByteArray? {
        val answer = AtomicReference<ByteArray?>(null)
        val done = CountDownLatch(1)
        val cancel = CancellationSignal()
        DnsResolver.getInstance().rawQuery(
            // The default network.
            null,
            name,
            DnsResolver.CLASS_IN,
            type,
            DnsResolver.FLAG_EMPTY,
            executor,
            cancel,
            object : DnsResolver.Callback<ByteArray> {
                override fun onAnswer(
                    result: ByteArray,
                    rcode: Int,
                ) {
                    answer.set(result)
                    done.countDown()
                }

                override fun onError(error: DnsResolver.DnsException) {
                    done.countDown()
                }
            },
        )
        if (!done.await(TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            cancel.cancel()
            return null
        }
        return answer.get()
    }

    private companion object {
        const val TIMEOUT_SECONDS = 5L
        val executor = Executors.newSingleThreadExecutor()
    }
}
