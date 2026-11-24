import MailcraneCore
import Security
import XCTest

@testable import Mailcrane

final class KeychainSecretStoreTests: XCTestCase {
    private var service = ""
    private var store = KeychainSecretStore()

    override func setUp() {
        super.setUp()
        service = "app.mailcrane.secrets.test.\(UUID().uuidString)"
        store = KeychainSecretStore(service: service)
    }

    override func tearDown() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service
        ]
        SecItemDelete(query as CFDictionary)
        super.tearDown()
    }

    func testPutGetDeleteRoundTrips() throws {
        let key = "account/1/password"
        try store.put(key: key, value: Data("pw-example".utf8))
        XCTAssertEqual(try store.get(key: key), Data("pw-example".utf8))

        try store.delete(key: key)
        XCTAssertNil(try store.get(key: key))
        // Deleting a missing key is not an error.
        try store.delete(key: key)
    }

    func testPutReplacesTheOldValue() throws {
        let key = "account/1/password"
        try store.put(key: key, value: Data("old-example".utf8))
        try store.put(key: key, value: Data("new-example".utf8))
        XCTAssertEqual(try store.get(key: key), Data("new-example".utf8))
    }

    func testKeysAreSeparate() throws {
        try store.put(key: "account/1/password", value: Data("a".utf8))
        XCTAssertNil(try store.get(key: "account/2/password"))
    }

    func testItemsAreThisDeviceOnlyAfterFirstUnlock() throws {
        let key = "account/1/password"
        try store.put(key: key, value: Data("pw-example".utf8))

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var result: CFTypeRef?
        XCTAssertEqual(SecItemCopyMatching(query as CFDictionary, &result), errSecSuccess)
        let attributes = try XCTUnwrap(result as? [String: Any])
        let accessible = try XCTUnwrap(attributes[kSecAttrAccessible as String] as? String)
        XCTAssertEqual(accessible, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
    }

    func testKeychainStatusesMapToSecretStoreErrors() {
        XCTAssertEqual(KeychainSecretStore.error(for: errSecInteractionNotAllowed), .Unavailable)
        XCTAssertEqual(KeychainSecretStore.error(for: errSecDecode), .Invalidated)
        XCTAssertEqual(KeychainSecretStore.error(for: errSecParam), .Other(reason: "OSStatus \(errSecParam)"))
    }
}
