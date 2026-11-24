import XCTest

@testable import Mailcrane

private struct FakeCoreService: CoreService {
    let greeting: String

    func hello() -> String {
        greeting
    }
}

final class ContentViewModelTests: XCTestCase {
    func testExposesGreetingFromCore() {
        let viewModel = ContentViewModel(
            core: FakeCoreService(greeting: "Hello from fake core")
        )

        XCTAssertEqual(viewModel.greeting, "Hello from fake core")
    }
}
