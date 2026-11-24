import MailcraneCore

private func coreHello() -> String {
    hello()
}

protocol CoreService {
    func hello() -> String
}

struct LiveCoreService: CoreService {
    func hello() -> String {
        coreHello()
    }
}
