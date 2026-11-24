import Foundation
import MailcraneCore
import Security

/// Keeps credentials in the iOS Keychain as generic-password items.
///
/// Items use `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, so background
/// fetch can read them once the device has been unlocked after a restart, and
/// they never go into backups or onto another device.
///
/// Every failure is reported as a `SecretStoreError`, never a crash, and no
/// key, value or status is logged.
final class KeychainSecretStore: PlatformSecretStore {
    static let defaultService = "app.mailcrane.secrets"

    private let service: String

    init(service: String = KeychainSecretStore.defaultService) {
        self.service = service
    }

    func put(key: String, value: Data) throws {
        let query = baseQuery(key: key)
        let attributes: [String: Any] = [
            kSecValueData as String: value,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        switch status {
        case errSecSuccess:
            return
        case errSecItemNotFound:
            let item = query.merging(attributes) { _, new in new }
            let added = SecItemAdd(item as CFDictionary, nil)
            guard added == errSecSuccess else { throw Self.error(for: added) }
        default:
            throw Self.error(for: status)
        }
    }

    func get(key: String) throws -> Data? {
        var query = baseQuery(key: key)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            guard let data = result as? Data else { throw SecretStoreError.Invalidated }
            return data
        case errSecItemNotFound:
            return nil
        default:
            throw Self.error(for: status)
        }
    }

    func delete(key: String) throws {
        let status = SecItemDelete(baseQuery(key: key) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw Self.error(for: status)
        }
    }

    private func baseQuery(key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key
        ]
    }

    /// Maps a Keychain status to a `SecretStoreError`. Only the numeric
    /// status is kept, never a value.
    static func error(for status: OSStatus) -> SecretStoreError {
        switch status {
        case errSecInteractionNotAllowed, errSecNotAvailable:
            // e.g. the device hasn't been unlocked since it restarted.
            return .Unavailable
        case errSecDecode, errSecAuthFailed:
            return .Invalidated
        default:
            return .Other(reason: "OSStatus \(status)")
        }
    }
}
