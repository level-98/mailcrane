package app.mailcrane.security

import android.content.Context
import android.os.Build
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_WEAK
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity

/**
 * The system unlock prompt: fingerprint or face, with the device's own PIN, pattern or password as
 * the fallback. Mailcrane keeps no PIN of its own.
 */
object DeviceAuth {
    // Strong biometrics combined with the device credential needs Android 11 (API 30); before
    // that the system only allows weak biometrics alongside it.
    private val authenticators: Int =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            BIOMETRIC_STRONG or DEVICE_CREDENTIAL
        } else {
            BIOMETRIC_WEAK or DEVICE_CREDENTIAL
        }

    /** Whether this device has a screen lock (or enrolled biometrics) the app lock can use. */
    fun available(context: Context): Boolean =
        BiometricManager.from(context).canAuthenticate(authenticators) == BiometricManager.BIOMETRIC_SUCCESS

    /**
     * Shows the prompt. [onResult] gets true after a successful unlock, false if it was cancelled or
     * failed for good (a single wrong fingerprint just lets the user try again).
     */
    fun prompt(
        activity: FragmentActivity,
        title: String,
        onResult: (Boolean) -> Unit,
    ) {
        val callback =
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = onResult(true)

                override fun onAuthenticationError(
                    errorCode: Int,
                    errString: CharSequence,
                ) = onResult(false)
            }
        val info =
            BiometricPrompt.PromptInfo
                .Builder()
                .setTitle(title)
                .setAllowedAuthenticators(authenticators)
                .build()
        BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback).authenticate(info)
    }
}
