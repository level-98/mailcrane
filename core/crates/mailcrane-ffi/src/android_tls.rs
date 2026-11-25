//! Android set-up for TLS certificate checks.
//!
//! The core checks server certificates with `rustls-platform-verifier`, which
//! on Android asks the system's trust manager through the JVM. That needs the
//! application `Context` before the first TLS connection; without it every
//! handshake fails (issue #243). The app calls
//! `app.mailcrane.core.TlsSetup.init(context)` once from
//! `Application.onCreate()`.
//!
//! JNI entry points are `unsafe` by nature (an exported, unmangled symbol), so
//! this module allows it locally. It does nothing but hand the context over.

#![allow(unsafe_code)]

use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JObject};
use jni::EnvUnowned;

/// `TlsSetup.init(context)`. Safe to call more than once; later calls do
/// nothing. A failure is thrown to Kotlin as a `RuntimeException`, and a
/// panic is caught rather than unwinding into the JVM.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_mailcrane_core_TlsSetup_nativeInit<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    unowned_env
        .with_env(|env| -> jni::errors::Result<()> {
            rustls_platform_verifier::android::init_with_env(env, context)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}
