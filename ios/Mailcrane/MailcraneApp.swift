import SwiftUI

@main
struct MailcraneApp: App {
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
