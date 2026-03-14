import Foundation
import AppKit
import ApplicationServices

struct HelperRequest: Decodable {
    let id: String?
    let action: String
    let payload: [String: JSONValue]?
}

struct HelperResponse: Encodable {
    let id: String?
    let ok: Bool
    let result: [String: JSONValue]?
    let meta: [String: JSONValue]?
    let error: String?
}

enum JSONValue: Codable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: JSONValue])
    case array([JSONValue])
    case null

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            throw DecodingError.typeMismatch(JSONValue.self, .init(codingPath: decoder.codingPath, debugDescription: "Unsupported JSON value"))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

@main
enum DuxNodeMacosAxHelper {
    static func main() {
        let input = FileHandle.standardInput
        let output = FileHandle.standardOutput
        let decoder = JSONDecoder()
        let encoder = JSONEncoder()
        encoder.outputFormatting = []

        while let line = readLine() {
            let data = Data(line.utf8)
            let response: HelperResponse
            do {
                let request = try decoder.decode(HelperRequest.self, from: data)
                response = handle(request: request)
            } catch {
                response = HelperResponse(id: nil, ok: false, result: nil, meta: nil, error: "invalid_request: \(error.localizedDescription)")
            }

            do {
                let data = try encoder.encode(response)
                if let line = String(data: data, encoding: .utf8) {
                    output.write(Data((line + "\n").utf8))
                }
            } catch {
                let fallback = "{\"id\":null,\"ok\":false,\"error\":\"encode_failed\"}\n"
                output.write(Data(fallback.utf8))
            }

            if input.availableData.isEmpty {
                fflush(stdout)
            }
        }
    }

    static func handle(request: HelperRequest) -> HelperResponse {
        switch request.action {
        case "ax.status":
            return accessibilityStatusResponse(id: request.id)
        case "app.activate":
            return activateApplicationResponse(id: request.id, payload: request.payload ?? [:])
        case "window.focus":
            return focusWindowResponse(id: request.id, payload: request.payload ?? [:])
        case "ax.tree":
            return axTreeResponse(id: request.id, payload: request.payload ?? [:])
        default:
            return HelperResponse(
                id: request.id,
                ok: false,
                result: nil,
                meta: ["scaffold": .bool(true)],
                error: "unsupported_action:\(request.action)"
            )
        }
    }

    static func accessibilityStatusResponse(id: String?) -> HelperResponse {
        let trusted = AXIsProcessTrusted()
        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "platform": .string("macos"),
                "helper": .string("swift-ax"),
                "ready": .bool(trusted),
                "trusted": .bool(trusted),
                "summary": .string(trusted ? "辅助功能已授权" : "辅助功能未授权"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func activateApplicationResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let app = resolveApplication(payload: payload) else {
            return HelperResponse(
                id: id,
                ok: false,
                result: nil,
                meta: ["scaffold": .bool(false)],
                error: "application_not_found"
            )
        }

        let success = app.activate(options: [.activateAllWindows, .activateIgnoringOtherApps])
        return HelperResponse(
            id: id,
            ok: success,
            result: success ? [
                "bundle_id": .string(app.bundleIdentifier ?? ""),
                "localized_name": .string(app.localizedName ?? ""),
                "process_id": .number(Double(app.processIdentifier)),
                "summary": .string("已激活应用 \(app.localizedName ?? app.bundleIdentifier ?? "")"),
            ] : nil,
            meta: ["scaffold": .bool(false)],
            error: success ? nil : "application_activate_failed"
        )
    }

    static func focusWindowResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let app = resolveApplication(payload: payload) else {
            return HelperResponse(
                id: id,
                ok: false,
                result: nil,
                meta: ["scaffold": .bool(false)],
                error: "application_not_found"
            )
        }

        let activated = app.activate(options: [.activateAllWindows, .activateIgnoringOtherApps])
        let windowTitle = stringValue(payload["window_title"])
        let focusedWindow = focusWindow(app: app, title: windowTitle)
        let success = activated && focusedWindow != nil
        return HelperResponse(
            id: id,
            ok: success,
            result: success ? [
                "bundle_id": .string(app.bundleIdentifier ?? ""),
                "localized_name": .string(app.localizedName ?? ""),
                "window_title": .string(windowTitleValue(focusedWindow) ?? windowTitle ?? ""),
                "summary": .string(windowTitle?.isEmpty == false ? "已聚焦窗口 \(windowTitleValue(focusedWindow) ?? windowTitle!)" : "已聚焦应用窗口"),
            ] : nil,
            meta: ["scaffold": .bool(false)],
            error: success ? nil : (activated ? "window_focus_failed" : "application_activate_failed")
        )
    }

    static func axTreeResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let app = resolveApplication(payload: payload) else {
            return HelperResponse(
                id: id,
                ok: false,
                result: nil,
                meta: ["scaffold": .bool(false)],
                error: "application_not_found"
            )
        }

        let windows = applicationWindows(app)
        let items: [JSONValue] = windows.enumerated().map { index, window in
            .object([
                "index": .number(Double(index)),
                "title": .string(windowTitleValue(window) ?? ""),
            ])
        }

        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "bundle_id": .string(app.bundleIdentifier ?? ""),
                "localized_name": .string(app.localizedName ?? ""),
                "windows": .array(items),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func resolveApplication(payload: [String: JSONValue]) -> NSRunningApplication? {
        if let bundleID = stringValue(payload["bundle_id"]), !bundleID.isEmpty {
            let matched = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
            if let app = matched.first(where: { !$0.isTerminated }) ?? matched.first {
                return app
            }
        }

        if let appName = stringValue(payload["app_name"]), !appName.isEmpty {
            let lowercased = appName.lowercased()
            let apps = NSWorkspace.shared.runningApplications.filter { app in
                guard !app.isTerminated else {
                    return false
                }
                let localized = app.localizedName?.lowercased() ?? ""
                let bundle = app.bundleIdentifier?.lowercased() ?? ""
                return localized == lowercased || bundle == lowercased || localized.contains(lowercased)
            }
            if let app = apps.first {
                return app
            }
        }

        return nil
    }

    static func stringValue(_ value: JSONValue?) -> String? {
        guard let value else {
            return nil
        }
        switch value {
        case .string(let string):
            return string
        case .number(let number):
            return String(number)
        case .bool(let bool):
            return bool ? "true" : "false"
        case .null:
            return nil
        case .object, .array:
            return nil
        }
    }

    static func focusWindow(app: NSRunningApplication, title: String?) -> AXUIElement? {
        let windows = applicationWindows(app)
        let target: AXUIElement?
        if let title, !title.isEmpty {
            let needle = title.lowercased()
            target = windows.first(where: { (windowTitleValue($0) ?? "").lowercased().contains(needle) })
        } else {
            target = windows.first
        }

        guard let target else {
            return nil
        }

        AXUIElementPerformAction(target, kAXRaiseAction as CFString)
        AXUIElementSetAttributeValue(target, kAXMainAttribute as CFString, kCFBooleanTrue)
        AXUIElementSetAttributeValue(target, kAXFocusedAttribute as CFString, kCFBooleanTrue)
        return target
    }

    static func applicationWindows(_ app: NSRunningApplication) -> [AXUIElement] {
        let element = AXUIElementCreateApplication(app.processIdentifier)
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(element, kAXWindowsAttribute as CFString, &value)
        guard status == .success, let value, CFGetTypeID(value) == CFArrayGetTypeID() else {
            return []
        }
        return value as? [AXUIElement] ?? []
    }

    static func windowTitleValue(_ window: AXUIElement?) -> String? {
        guard let window else {
            return nil
        }
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(window, kAXTitleAttribute as CFString, &value)
        guard status == .success, let value else {
            return nil
        }
        return value as? String
    }
}
