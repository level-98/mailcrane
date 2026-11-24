import XCTest

final class ScreenshotTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testLightModeScreenshot() throws {
        let app = XCUIApplication()
        app.launch()
        attachScreenshot(app)
    }

    func testDarkModeScreenshot() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-UITestDarkMode"]
        app.launch()
        attachScreenshot(app)
    }

    func testLargeTextScreenshot() throws {
        let app = XCUIApplication()
        app.launchArguments = [
            "-UIPreferredContentSizeCategoryName",
            "UICTContentSizeCategoryAccessibilityExtraExtraExtraLarge"
        ]
        app.launch()
        attachScreenshot(app)
    }

    private func attachScreenshot(_ app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
