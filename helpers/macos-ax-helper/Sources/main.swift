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
            throw DecodingError.typeMismatch(
                JSONValue.self,
                .init(codingPath: decoder.codingPath, debugDescription: "Unsupported JSON value")
            )
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

struct ElementDescriptor {
    let windowIndex: Int
    let path: [Int]
}

struct ElementLocator {
    let title: String?
    let value: String?
    let role: String?
    let subrole: String?
    let description: String?
    let text: String?
    let index: Int?
    let maxDepth: Int
}

struct ElementMatch {
    let element: AXUIElement
    let descriptor: ElementDescriptor
    let windowTitle: String
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
                response = HelperResponse(
                    id: nil,
                    ok: false,
                    result: nil,
                    meta: nil,
                    error: "invalid_request: \(error.localizedDescription)"
                )
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
        let payload = request.payload ?? [:]
        switch request.action {
        case "ui.status", "ax.status":
            return accessibilityStatusResponse(id: request.id)
        case "app.activate":
            return activateApplicationResponse(id: request.id, payload: payload)
        case "window.focus":
            return focusWindowResponse(id: request.id, payload: payload)
        case "ui.tree", "ax.tree":
            return axTreeResponse(id: request.id, payload: payload)
        case "ui.find":
            return uiFindResponse(id: request.id, payload: payload)
        case "ui.read":
            return uiReadResponse(id: request.id, payload: payload)
        case "ui.write":
            return uiWriteResponse(id: request.id, payload: payload)
        case "ui.invoke":
            return uiInvokeResponse(id: request.id, payload: payload)
        case "ui.focus":
            return uiFocusResponse(id: request.id, payload: payload)
        case "ui.click":
            return uiClickResponse(id: request.id, payload: payload)
        case "ui.type_native":
            return uiTypeNativeResponse(id: request.id, payload: payload)
        case "ui.keypress":
            return uiKeypressResponse(id: request.id, payload: payload)
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
            return errorResponse(id: id, error: "application_not_found")
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
            return errorResponse(id: id, error: "application_not_found")
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
                "summary": .string(windowTitle?.isEmpty == false
                    ? "已聚焦窗口 \(windowTitleValue(focusedWindow) ?? windowTitle!)"
                    : "已聚焦应用窗口"),
            ] : nil,
            meta: ["scaffold": .bool(false)],
            error: success ? nil : (activated ? "window_focus_failed" : "application_activate_failed")
        )
    }

    static func axTreeResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let app = resolveApplication(payload: payload) else {
            return errorResponse(id: id, error: "application_not_found")
        }

        let windows = applicationWindows(app)
        let maxDepth = max(0, intValue(payload["max_depth"]) ?? 1)
        let items: [JSONValue] = windows.enumerated().map { index, window in
            .object(serializeElement(
                window,
                descriptor: ElementDescriptor(windowIndex: index, path: []),
                windowTitle: windowTitleValue(window) ?? "",
                depth: 0,
                includeChildren: maxDepth > 0,
                maxDepth: maxDepth
            ))
        }

        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "bundle_id": .string(app.bundleIdentifier ?? ""),
                "localized_name": .string(app.localizedName ?? ""),
                "windows": .array(items),
                "summary": .string("已获取窗口树"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func uiFindResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let app = resolveApplication(payload: payload) else {
            return errorResponse(id: id, error: "application_not_found")
        }
        guard let locator = locatorFromPayload(payload) else {
            return errorResponse(id: id, error: "locator_required")
        }

        let matches = findElements(app: app, payload: payload, locator: locator)
        guard !matches.isEmpty else {
            return errorResponse(id: id, error: "element_not_found")
        }

        let limit = max(1, intValue(payload["limit"]) ?? 5)
        let serialized = matches.prefix(limit).map { match in
            JSONValue.object(
                serializeElement(match.element, descriptor: match.descriptor, windowTitle: match.windowTitle)
            )
        }

        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "match": serialized.first ?? .null,
                "matches": .array(Array(serialized)),
                "count": .number(Double(matches.count)),
                "summary": .string("已找到 \(matches.count) 个匹配控件"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func uiReadResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let resolved = resolveTargetElement(payload: payload) else {
            return errorResponse(id: id, error: "element_not_found")
        }
        let serialized = serializeElement(
            resolved.element,
            descriptor: resolved.descriptor,
            windowTitle: resolved.windowTitle
        )
        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "element": .object(serialized),
                "summary": .string("已读取控件信息"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func uiWriteResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let resolved = resolveTargetElement(payload: payload) else {
            return errorResponse(id: id, error: "element_not_found")
        }

        let text = stringValue(payload["text"]) ?? stringValue(payload["value"]) ?? ""
        if text.isEmpty {
            return errorResponse(id: id, error: "value_required")
        }

        let status = AXUIElementSetAttributeValue(resolved.element, kAXValueAttribute as CFString, text as CFTypeRef)
        guard status == .success else {
            return errorResponse(id: id, error: "element_write_failed")
        }

        let serialized = serializeElement(
            resolved.element,
            descriptor: resolved.descriptor,
            windowTitle: resolved.windowTitle
        )
        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "element": .object(serialized),
                "written_value": .string(text),
                "summary": .string("已写入控件内容"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func uiInvokeResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let resolved = resolveTargetElement(payload: payload) else {
            return errorResponse(id: id, error: "element_not_found")
        }

        let actionName = normalizedActionName(stringValue(payload["invoke_action"]))
        let status = AXUIElementPerformAction(resolved.element, actionName as CFString)
        guard status == .success else {
            return errorResponse(id: id, error: "element_invoke_failed")
        }

        let serialized = serializeElement(
            resolved.element,
            descriptor: resolved.descriptor,
            windowTitle: resolved.windowTitle
        )
        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "element": .object(serialized),
                "invoke_action": .string(actionName),
                "summary": .string("已执行控件动作"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func uiFocusResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        guard let resolved = resolveTargetElement(payload: payload) else {
            return errorResponse(id: id, error: "element_not_found")
        }
        let ok = focusElement(resolved.element)
        guard ok else {
            return errorResponse(id: id, error: "element_focus_failed")
        }
        let serialized = serializeElement(
            resolved.element,
            descriptor: resolved.descriptor,
            windowTitle: resolved.windowTitle
        )
        return HelperResponse(
            id: id,
            ok: true,
            result: [
                "element": .object(serialized),
                "summary": .string("已聚焦控件"),
            ],
            meta: ["scaffold": .bool(false)],
            error: nil
        )
    }

    static func uiClickResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        let point: CGPoint?
        if let resolved = resolveTargetElement(payload: payload) {
            point = clickPointForElement(resolved.element)
        } else if let x = doubleValue(payload["x"]), let y = doubleValue(payload["y"]) {
            point = CGPoint(x: x, y: y)
        } else {
            point = nil
        }

        guard let point else {
            return errorResponse(id: id, error: "click_target_required")
        }

        do {
            try postLeftClick(at: point)
            return HelperResponse(
                id: id,
                ok: true,
                result: [
                    "x": .number(point.x),
                    "y": .number(point.y),
                    "summary": .string("已发送点击事件"),
                ],
                meta: ["scaffold": .bool(false)],
                error: nil
            )
        } catch {
            return errorResponse(id: id, error: "click_failed")
        }
    }

    static func uiTypeNativeResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        if let resolved = resolveTargetElement(payload: payload) {
            _ = focusElement(resolved.element)
        }

        let text = stringValue(payload["text"]) ?? ""
        if text.isEmpty {
            return errorResponse(id: id, error: "value_required")
        }

        do {
            try postUnicodeText(text)
            return HelperResponse(
                id: id,
                ok: true,
                result: [
                    "typed_text": .string(text),
                    "summary": .string("已发送原生键盘输入"),
                ],
                meta: ["scaffold": .bool(false)],
                error: nil
            )
        } catch {
            return errorResponse(id: id, error: "native_type_failed")
        }
    }

    static func uiKeypressResponse(id: String?, payload: [String: JSONValue]) -> HelperResponse {
        if let resolved = resolveTargetElement(payload: payload) {
            _ = focusElement(resolved.element)
        }

        let key = nonEmpty(stringValue(payload["key"])) ?? "return"
        let modifiers = arrayValue(payload["modifiers"]).compactMap(stringValue)
        do {
            try postKeyPress(key: key, modifiers: modifiers)
            return HelperResponse(
                id: id,
                ok: true,
                result: [
                    "key": .string(key),
                    "modifiers": .array(modifiers.map { .string($0) }),
                    "summary": .string("已发送按键事件"),
                ],
                meta: ["scaffold": .bool(false)],
                error: nil
            )
        } catch {
            return errorResponse(id: id, error: "keypress_failed")
        }
    }

    static func resolveTargetElement(payload: [String: JSONValue]) -> ElementMatch? {
        guard let app = resolveApplication(payload: payload) else {
            return nil
        }

        if let descriptor = descriptorFromPayload(payload),
           let element = resolveElement(app: app, descriptor: descriptor) {
            return ElementMatch(
                element: element,
                descriptor: descriptor,
                windowTitle: windowTitleValue(applicationWindows(app)[descriptor.windowIndex]) ?? ""
            )
        }

        guard let locator = locatorFromPayload(payload) else {
            return nil
        }
        return findElements(app: app, payload: payload, locator: locator).first
    }

    static func findElements(app: NSRunningApplication, payload: [String: JSONValue], locator: ElementLocator) -> [ElementMatch] {
        var matches: [ElementMatch] = []
        for root in searchRoots(app: app, payload: payload) {
            collectMatches(
                element: root.element,
                descriptor: root.descriptor,
                locator: locator,
                windowTitle: root.windowTitle,
                depth: 0,
                output: &matches
            )
        }
        if let index = locator.index, index >= 0, index < matches.count {
            return [matches[index]]
        }
        return matches
    }

    static func searchRoots(app: NSRunningApplication, payload: [String: JSONValue]) -> [ElementMatch] {
        if let within = descriptorFromValue(payload["within"]),
           let element = resolveElement(app: app, descriptor: within) {
            let windows = applicationWindows(app)
            let title = (within.windowIndex >= 0 && within.windowIndex < windows.count)
                ? (windowTitleValue(windows[within.windowIndex]) ?? "")
                : ""
            return [ElementMatch(element: element, descriptor: within, windowTitle: title)]
        }

        return filteredWindows(app: app, payload: payload).map { windowIndex, window in
            ElementMatch(
                element: window,
                descriptor: ElementDescriptor(windowIndex: windowIndex, path: []),
                windowTitle: windowTitleValue(window) ?? ""
            )
        }
    }

    static func collectMatches(
        element: AXUIElement,
        descriptor: ElementDescriptor,
        locator: ElementLocator,
        windowTitle: String,
        depth: Int,
        output: inout [ElementMatch]
    ) {
        if elementMatches(element, locator: locator) {
            output.append(ElementMatch(element: element, descriptor: descriptor, windowTitle: windowTitle))
        }
        if depth >= locator.maxDepth {
            return
        }
        let children = childrenOfElement(element)
        for (index, child) in children.enumerated() {
            collectMatches(
                element: child,
                descriptor: ElementDescriptor(windowIndex: descriptor.windowIndex, path: descriptor.path + [index]),
                locator: locator,
                windowTitle: windowTitle,
                depth: depth + 1,
                output: &output
            )
        }
    }

    static func elementMatches(_ element: AXUIElement, locator: ElementLocator) -> Bool {
        if let role = locator.role, !matchesSubstring(roleValue(element), needle: role) {
            return false
        }
        if let subrole = locator.subrole, !matchesSubstring(subroleValue(element), needle: subrole) {
            return false
        }
        if let title = locator.title, !matchesSubstring(titleValue(element), needle: title) {
            return false
        }
        if let value = locator.value, !matchesSubstring(valueString(element), needle: value) {
            return false
        }
        if let description = locator.description, !matchesSubstring(descriptionValue(element), needle: description) {
            return false
        }
        if let text = locator.text {
            let haystacks = [
                titleValue(element),
                valueString(element),
                descriptionValue(element),
                roleValue(element),
                subroleValue(element),
            ]
            let anyMatch = haystacks.contains { matchesSubstring($0, needle: text) }
            if !anyMatch {
                return false
            }
        }
        return true
    }

    static func filteredWindows(app: NSRunningApplication, payload: [String: JSONValue]) -> [(Int, AXUIElement)] {
        let windows = applicationWindows(app)
        if let windowTitle = stringValue(payload["window_title"]), !windowTitle.isEmpty {
            let needle = windowTitle.lowercased()
            return windows.enumerated().filter { (_, window) in
                (windowTitleValue(window) ?? "").lowercased().contains(needle)
            }
        }
        return windows.enumerated().map { ($0.offset, $0.element) }
    }

    static func resolveElement(app: NSRunningApplication, descriptor: ElementDescriptor) -> AXUIElement? {
        let windows = applicationWindows(app)
        guard descriptor.windowIndex >= 0, descriptor.windowIndex < windows.count else {
            return nil
        }
        var current = windows[descriptor.windowIndex]
        if descriptor.path.isEmpty {
            return current
        }
        for index in descriptor.path {
            let children = childrenOfElement(current)
            guard index >= 0, index < children.count else {
                return nil
            }
            current = children[index]
        }
        return current
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

    static func locatorFromPayload(_ payload: [String: JSONValue]) -> ElementLocator? {
        let source = objectValue(payload["locator"]) ?? payload
        let title = nonEmpty(stringValue(source["title"]))
        let value = nonEmpty(stringValue(source["value"]))
        let role = nonEmpty(stringValue(source["role"]))
        let subrole = nonEmpty(stringValue(source["subrole"]))
        let description = nonEmpty(stringValue(source["description"]))
        let text = nonEmpty(stringValue(source["text"]))
        let index = intValue(source["index"])
        let maxDepth = max(0, min(20, intValue(source["max_depth"]) ?? intValue(payload["max_depth"]) ?? 10))
        if title == nil && value == nil && role == nil && subrole == nil && description == nil && text == nil {
            return nil
        }
        return ElementLocator(
            title: title,
            value: value,
            role: role,
            subrole: subrole,
            description: description,
            text: text,
            index: index,
            maxDepth: maxDepth
        )
    }

    static func descriptorFromPayload(_ payload: [String: JSONValue]) -> ElementDescriptor? {
        guard let object = objectValue(payload["element"]) ?? objectValue(payload["descriptor"]) else {
            return nil
        }
        return descriptorFromObject(object)
    }

    static func descriptorFromValue(_ value: JSONValue?) -> ElementDescriptor? {
        guard let object = objectValue(value) else {
            return nil
        }
        return descriptorFromObject(object)
    }

    static func descriptorFromObject(_ object: [String: JSONValue]) -> ElementDescriptor? {
        guard let windowIndex = intValue(object["window_index"]) else {
            return nil
        }
        let path = arrayValue(object["path"]).compactMap(intValue)
        return ElementDescriptor(windowIndex: windowIndex, path: path)
    }

    static func serializeElement(
        _ element: AXUIElement,
        descriptor: ElementDescriptor,
        windowTitle: String,
        depth: Int = 0,
        includeChildren: Bool = false,
        maxDepth: Int = 0
    ) -> [String: JSONValue] {
        var result: [String: JSONValue] = [
            "window_title": .string(windowTitle),
            "role": .string(roleValue(element) ?? ""),
            "subrole": .string(subroleValue(element) ?? ""),
            "title": .string(titleValue(element) ?? ""),
            "value": .string(valueString(element) ?? ""),
            "description": .string(descriptionValue(element) ?? ""),
            "enabled": .bool(boolAttribute(element, attribute: kAXEnabledAttribute as CFString)),
            "focused": .bool(boolAttribute(element, attribute: kAXFocusedAttribute as CFString)),
            "children_count": .number(Double(childrenOfElement(element).count)),
            "descriptor": .object([
                "window_index": .number(Double(descriptor.windowIndex)),
                "path": .array(descriptor.path.map { .number(Double($0)) }),
            ]),
        ]

        if let frame = frameOfElement(element) {
            result["frame"] = .object([
                "x": .number(frame.origin.x),
                "y": .number(frame.origin.y),
                "width": .number(frame.size.width),
                "height": .number(frame.size.height),
                "center_x": .number(frame.midX),
                "center_y": .number(frame.midY),
            ])
        }

        if includeChildren && depth < maxDepth {
            let children = childrenOfElement(element).enumerated().map { index, child in
                JSONValue.object(
                    serializeElement(
                        child,
                        descriptor: ElementDescriptor(windowIndex: descriptor.windowIndex, path: descriptor.path + [index]),
                        windowTitle: windowTitle,
                        depth: depth + 1,
                        includeChildren: true,
                        maxDepth: maxDepth
                    )
                )
            }
            result["children"] = .array(children)
        }

        return result
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

    static func focusElement(_ element: AXUIElement) -> Bool {
        let focused = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
        if focused == .success {
            return true
        }
        let action = AXUIElementPerformAction(element, kAXPressAction as CFString)
        return action == .success
    }

    static func clickPointForElement(_ element: AXUIElement) -> CGPoint? {
        guard let frame = frameOfElement(element) else {
            return nil
        }
        return CGPoint(x: frame.midX, y: frame.midY)
    }

    static func frameOfElement(_ element: AXUIElement) -> CGRect? {
        guard let positionValue = copyAttributeValue(element, attribute: kAXPositionAttribute as CFString),
              let sizeValue = copyAttributeValue(element, attribute: kAXSizeAttribute as CFString),
              let position = axPoint(positionValue),
              let size = axSize(sizeValue) else {
            return nil
        }
        return CGRect(origin: position, size: size)
    }

    static func applicationWindows(_ app: NSRunningApplication) -> [AXUIElement] {
        let element = AXUIElementCreateApplication(app.processIdentifier)
        guard let value = copyAttributeValue(element, attribute: kAXWindowsAttribute as CFString) else {
            return []
        }
        return value as? [AXUIElement] ?? []
    }

    static func childrenOfElement(_ element: AXUIElement) -> [AXUIElement] {
        guard let value = copyAttributeValue(element, attribute: kAXChildrenAttribute as CFString) else {
            return []
        }
        return value as? [AXUIElement] ?? []
    }

    static func roleValue(_ element: AXUIElement) -> String? {
        return copyAttributeValue(element, attribute: kAXRoleAttribute as CFString) as? String
    }

    static func subroleValue(_ element: AXUIElement) -> String? {
        return copyAttributeValue(element, attribute: kAXSubroleAttribute as CFString) as? String
    }

    static func titleValue(_ element: AXUIElement) -> String? {
        return copyAttributeValue(element, attribute: kAXTitleAttribute as CFString) as? String
    }

    static func descriptionValue(_ element: AXUIElement) -> String? {
        return copyAttributeValue(element, attribute: kAXDescriptionAttribute as CFString) as? String
    }

    static func windowTitleValue(_ window: AXUIElement?) -> String? {
        guard let window else {
            return nil
        }
        return titleValue(window)
    }

    static func valueString(_ element: AXUIElement) -> String? {
        guard let value = copyAttributeValue(element, attribute: kAXValueAttribute as CFString) else {
            return nil
        }
        return cfValueToString(value)
    }

    static func boolAttribute(_ element: AXUIElement, attribute: CFString) -> Bool {
        guard let value = copyAttributeValue(element, attribute: attribute) else {
            return false
        }
        if let boolValue = value as? Bool {
            return boolValue
        }
        if let number = value as? NSNumber {
            return number.boolValue
        }
        return false
    }

    static func copyAttributeValue(_ element: AXUIElement, attribute: CFString) -> AnyObject? {
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(element, attribute, &value)
        guard status == .success, let value else {
            return nil
        }
        return value
    }

    static func cfValueToString(_ value: AnyObject) -> String? {
        if let string = value as? String {
            return string
        }
        if let attributed = value as? NSAttributedString {
            return attributed.string
        }
        if let number = value as? NSNumber {
            return number.stringValue
        }
        if CFGetTypeID(value) == AXUIElementGetTypeID() {
            return nil
        }
        return String(describing: value)
    }

    static func axPoint(_ value: AnyObject) -> CGPoint? {
        guard CFGetTypeID(value) == AXValueGetTypeID() else {
            return nil
        }
        let axValue = unsafeBitCast(value, to: AXValue.self)
        guard AXValueGetType(axValue) == .cgPoint else {
            return nil
        }
        var point = CGPoint.zero
        return AXValueGetValue(axValue, .cgPoint, &point) ? point : nil
    }

    static func axSize(_ value: AnyObject) -> CGSize? {
        guard CFGetTypeID(value) == AXValueGetTypeID() else {
            return nil
        }
        let axValue = unsafeBitCast(value, to: AXValue.self)
        guard AXValueGetType(axValue) == .cgSize else {
            return nil
        }
        var size = CGSize.zero
        return AXValueGetValue(axValue, .cgSize, &size) ? size : nil
    }

    static func normalizedActionName(_ value: String?) -> String {
        switch (value ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "confirm":
            return kAXConfirmAction as String
        case "raise":
            return kAXRaiseAction as String
        case "show_menu", "showmenu":
            return kAXShowMenuAction as String
        default:
            return kAXPressAction as String
        }
    }

    static func matchesSubstring(_ haystack: String?, needle: String) -> Bool {
        guard let haystack, !haystack.isEmpty else {
            return false
        }
        return haystack.lowercased().contains(needle.lowercased())
    }

    static func stringValue(_ value: JSONValue?) -> String? {
        guard let value else {
            return nil
        }
        switch value {
        case .string(let string):
            return string
        case .number(let number):
            if number.rounded() == number {
                return String(Int(number))
            }
            return String(number)
        case .bool(let bool):
            return bool ? "true" : "false"
        case .null:
            return nil
        case .object, .array:
            return nil
        }
    }

    static func intValue(_ value: JSONValue?) -> Int? {
        guard let value else {
            return nil
        }
        switch value {
        case .number(let number):
            return Int(number)
        case .string(let string):
            return Int(string)
        default:
            return nil
        }
    }

    static func doubleValue(_ value: JSONValue?) -> Double? {
        guard let value else {
            return nil
        }
        switch value {
        case .number(let number):
            return number
        case .string(let string):
            return Double(string)
        default:
            return nil
        }
    }

    static func objectValue(_ value: JSONValue?) -> [String: JSONValue]? {
        guard case .object(let object)? = value else {
            return nil
        }
        return object
    }

    static func arrayValue(_ value: JSONValue?) -> [JSONValue] {
        guard case .array(let array)? = value else {
            return []
        }
        return array
    }

    static func nonEmpty(_ value: String?) -> String? {
        guard let value else {
            return nil
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    static func errorResponse(id: String?, error: String) -> HelperResponse {
        return HelperResponse(
            id: id,
            ok: false,
            result: nil,
            meta: ["scaffold": .bool(false)],
            error: error
        )
    }

    static func postUnicodeText(_ text: String) throws {
        guard let source = CGEventSource(stateID: .combinedSessionState) else {
            throw NSError(domain: "dux.ax", code: 1)
        }
        let scalars = Array(text.utf16)
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else {
            throw NSError(domain: "dux.ax", code: 2)
        }
        down.keyboardSetUnicodeString(stringLength: scalars.count, unicodeString: scalars)
        up.keyboardSetUnicodeString(stringLength: scalars.count, unicodeString: scalars)
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }

    static func postKeyPress(key: String, modifiers: [String]) throws {
        guard let source = CGEventSource(stateID: .combinedSessionState) else {
            throw NSError(domain: "dux.ax", code: 3)
        }
        let mapping = keyMapping(key)
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: mapping.keyCode, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: mapping.keyCode, keyDown: false) else {
            throw NSError(domain: "dux.ax", code: 4)
        }
        let flags = eventFlags(modifiers).union(mapping.flags)
        down.flags = flags
        up.flags = flags
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }

    static func postLeftClick(at point: CGPoint) throws {
        guard let source = CGEventSource(stateID: .combinedSessionState),
              let move = CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left),
              let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left),
              let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left) else {
            throw NSError(domain: "dux.ax", code: 5)
        }
        move.post(tap: .cghidEventTap)
        usleep(40_000)
        down.post(tap: .cghidEventTap)
        usleep(35_000)
        up.post(tap: .cghidEventTap)
    }

    static func eventFlags(_ modifiers: [String]) -> CGEventFlags {
        modifiers.reduce([]) { partial, item in
            switch item.lowercased() {
            case "command", "cmd":
                return partial.union(.maskCommand)
            case "shift":
                return partial.union(.maskShift)
            case "option", "alt":
                return partial.union(.maskAlternate)
            case "control", "ctrl":
                return partial.union(.maskControl)
            default:
                return partial
            }
        }
    }

    static func keyMapping(_ key: String) -> (keyCode: CGKeyCode, flags: CGEventFlags) {
        switch key.lowercased() {
        case "return", "enter":
            return (36, [])
        case "delete", "backspace":
            return (51, [])
        case "escape", "esc":
            return (53, [])
        case "tab":
            return (48, [])
        case "space":
            return (49, [])
        case "down", "arrowdown", "down_arrow":
            return (125, [])
        case "up", "arrowup", "up_arrow":
            return (126, [])
        case "left", "arrowleft", "left_arrow":
            return (123, [])
        case "right", "arrowright", "right_arrow":
            return (124, [])
        case "a":
            return (0, [])
        case "c":
            return (8, [])
        case "v":
            return (9, [])
        case "x":
            return (7, [])
        default:
            return (36, [])
        }
    }
}
