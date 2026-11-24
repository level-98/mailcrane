import Observation

@Observable
final class ContentViewModel {
    private(set) var greeting: String

    init(core: CoreService) {
        greeting = core.hello()
    }
}
