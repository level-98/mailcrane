import MailcraneCore
import SwiftUI

@main
struct MailcraneApp: App {
    init() {
        // The core reads and writes credentials only through this store.
        installSecretStore(store: KeychainSecretStore())
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .preferredColorScheme(
                    ProcessInfo.processInfo.arguments.contains("-UITestDarkMode")
                        ? .dark
                        : nil
                )
        }
    }
}
