// Run with scripts/test-native-shell.sh on macOS 26+. No database or visible
// window is needed; exercise real AppKit ownership, layout and delegate paths.
import AppKit

@main
struct NativeShellTests {
    static func main() {
        _ = NSApplication.shared
        guard shellSupported() else {
            print("Native shell lifecycle: skipped (requires macOS 26 and Xcode 26)")
            return
        }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
                              styleMask: [.titled, .resizable, .closable, .fullSizeContentView], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let root = window.contentView!
        let renderer = NSView(frame: root.bounds)
        root.addSubview(renderer)
        let originalStyle = window.toolbarStyle
        for _ in 0..<3 {
            let shell = shellCreate(Unmanaged.passUnretained(renderer).toOpaque())!
            assert(window.toolbar?.isVisible == false, "Do not flash controls before vault state arrives")
            let logo = #"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M2 2h20v20H2z"/></svg>"#
            let snapshot: [String: Any] = [
                "session": "fixture", "connections": [["id": "fixture", "label": "QA", "engine": "PostgreSQL", "logo": logo], ["id": "other", "label": "Other connection", "engine": "Redis", "logo": logo]],
                "sidebar": true, "canToggleSidebar": true, "connected": true, "unlocked": true, "dark": false,
                "systemAppearance": true, "modal": false, "search": "", "status": "Explorer · 80",
                "rows": (0..<80).map { ["id": "table-\($0)", "label": "Table \($0)", "symbol": "tablecells"] },
                "selected": ["table-3"], "schemas": [], "databases": [], "tableMenu": [], "explorerMenu": []
            ]
            let data = try! JSONSerialization.data(withJSONObject: snapshot)
            data.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, data.count) }
            window.contentView!.layoutSubtreeIfNeeded()
            let split = window.contentViewController as! NSSplitViewController
            assert(split.splitViewItems.count == 2)
            assert(!split.splitViewItems[0].isCollapsed)
            let scroller = split.splitViewItems[0].viewController.view as! NSScrollView
            let table = scroller.documentView as! NSTableView
            assert(!scroller.drawsBackground && table.numberOfRows == 80)
            assert(table.selectedRow == 3)
            assert(window.toolbar != nil && renderer.window === window)
            assert(renderer.superview === root, "Keep GPUI's AccessKit root")
            let connectionItems = window.toolbar!.items.filter { $0.itemIdentifier.rawValue.hasPrefix("connection:") }
            assert(connectionItems.count == 2, "Each connection has a visible tab")
            let addButton = window.toolbar!.items.first { $0.itemIdentifier.rawValue == "newConnection" }!.view as! NSButton
            assert(addButton.intrinsicContentSize.width <= 32 || addButton.constraints.contains { $0.firstAttribute == .width && $0.constant == 32 }, "Add control uses the compact toolbar size")
            addButton.performClick(nil)
            func tabFill(_ index: Int) -> CGFloat {
                (connectionItems[index].view as! NSStackView).layer!.backgroundColor!.alpha
            }
            assert(abs(tabFill(0) - 0.16) < 0.001 && tabFill(1) == 0, "Light mode has a distinct selected connection")
            let selectedLayer = (connectionItems[0].view as! NSStackView).layer!
            assert(abs(selectedLayer.borderColor!.alpha - 0.25) < 0.001, "Light mode selection has a visible edge")
            let fill = NSColor(cgColor: selectedLayer.backgroundColor!)!.usingColorSpace(.deviceRGB)!
            assert(fill.redComponent < 0.01, "Light selection uses dark ink regardless of system appearance")
            var switched = snapshot
            switched["session"] = "other"
            let switchedData = try! JSONSerialization.data(withJSONObject: switched)
            switchedData.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, switchedData.count) }
            assert(tabFill(0) == 0 && tabFill(1) > 0, "Selection highlight follows a tab switch")
            data.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, data.count) }
            assert(tabFill(0) > 0 && tabFill(1) == 0, "Selection highlight moves back")
            let selectedButton = (connectionItems[0].view as! NSStackView).arrangedSubviews.first as! NSButton
            let inactiveButton = (connectionItems[1].view as! NSStackView).arrangedSubviews.first as! NSButton
            func titleBrightness(_ button: NSButton) -> CGFloat {
                let color = button.attributedTitle.attribute(.foregroundColor, at: 0, effectiveRange: nil) as! NSColor
                return color.usingColorSpace(.deviceRGB)!.redComponent
            }
            assert(titleBrightness(inactiveButton) < 0.2, "Inactive connection stays legible in light mode")
            var darkSnapshot = snapshot
            darkSnapshot["dark"] = true
            let darkData = try! JSONSerialization.data(withJSONObject: darkSnapshot)
            darkData.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, darkData.count) }
            assert(titleBrightness(inactiveButton) > 0.9, "Inactive connection stays legible in dark mode")
            assert(abs(tabFill(0) - 0.10) < 0.001, "Dark selection keeps its existing treatment")
            data.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, data.count) }
            assert(selectedButton.image?.isTemplate == true && selectedButton.image?.size == NSSize(width: 16, height: 16), "Use the embedded database logo")
            assert(selectedButton.accessibilityLabel() == "QA, PostgreSQL")
            #if compiler(>=6.2)
            if #available(macOS 26.0, *) {
                let nativeMenu = testNativeContextMenu(shell, [
                    ["label": "Open", "index": 0, "enabled": true],
                    ["separator": true],
                    ["label": "Copy", "index": 2, "enabled": true, "children": [
                        ["label": "JSON", "index": 3, "enabled": true, "checked": true],
                        ["label": "Disabled", "index": 4, "enabled": false]
                    ]]
                ])
                assert(nativeMenu.items.count == 3 && nativeMenu.items[1].isSeparatorItem)
                let submenu = nativeMenu.items[2].submenu!
                assert(submenu.items[0].state == .on && !submenu.items[1].isEnabled)
                NSApplication.shared.sendAction(submenu.items[0].action!, to: submenu.items[0].target, from: submenu.items[0])
                assert(testNativeContextSelection(shell) == [2, 3], "Nested native selection keeps the original item path")
            }
            #endif
            let otherTab = connectionItems[1].view as! NSStackView
            assert(!otherTab.arrangedSubviews.contains { $0 is NSPopUpButton }, "Connection tabs must not regress to a popup")
            assert(otherTab.arrangedSubviews.first is NSButton && otherTab.arrangedSubviews.last is NSButton)
            let connectionContext = (otherTab.arrangedSubviews.first as! NSButton).menu!
            connectionContext.delegate?.menuNeedsUpdate?(connectionContext)
            assert(connectionContext.items.map(\.title) == ["Close connection", "Close other connections"])
            // Drag the first tab onto the second: a native mouse loop, no click.
            // Toolbar items only get frames once the window is ordered in.
            window.setFrameOrigin(NSPoint(x: -20000, y: -20000))
            window.orderFrontRegardless()
            window.contentView!.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            let firstSelect = (connectionItems[0].view as! NSStackView).arrangedSubviews.first as! NSButton
            let start = firstSelect.convert(NSPoint(x: firstSelect.bounds.midX, y: firstSelect.bounds.midY), to: nil)
            let end = NSPoint(x: otherTab.convert(otherTab.bounds, to: nil).midX, y: start.y)
            func mouse(_ type: NSEvent.EventType, _ at: NSPoint) -> NSEvent {
                NSEvent.mouseEvent(with: type, location: at, modifierFlags: [], timestamp: 0,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
            }
            window.postEvent(mouse(.leftMouseDragged, NSPoint(x: (start.x + end.x) / 2, y: start.y)), atStart: false)
            window.postEvent(mouse(.leftMouseDragged, end), atStart: false)
            window.postEvent(mouse(.leftMouseUp, end), atStart: false)
            firstSelect.mouseDown(with: mouse(.leftMouseDown, start))
            assert(window.childWindows?.isEmpty ?? true, "Drag ghost must not outlive the drag")
            (otherTab.arrangedSubviews.last as! NSButton).performClick(nil)
            let before = renderer.convert(renderer.bounds, to: nil)
            shellScroll(shell, 400, 1200)
            let after = renderer.convert(renderer.bounds, to: nil)
            assert(before == after, "Logical scrolling must not move GPUI's window coordinate origin: \(before) -> \(after)")
            table.selectRowIndexes(IndexSet(integer: 7), byExtendingSelection: false)
            let queued = shellDrain(shell)!
            let commands = try! JSONSerialization.jsonObject(with: Data(String(cString: queued).utf8)) as! [[String: Any]]
            free(queued)
            assert(commands.contains { $0["kind"] as? String == "geometry" && ($0["left"] as? Double ?? 0) >= 240 }, "Extended backdrop must not let the sidebar cover content")
            assert(commands.contains { $0["kind"] as? String == "newConnection" }, "Compact add control sends the same action")
            assert(commands.contains { $0["kind"] as? String == "closeConnection" && $0["id"] as? String == "other" })
            assert(commands.contains { $0["kind"] as? String == "moveConnection" && $0["id"] as? String == "fixture" && $0["target"] as? String == "other" })
            assert(!commands.contains { $0["kind"] as? String == "connection" }, "A drag must not also select the tab")
            assert(commands.contains { $0["kind"] as? String == "selection" && ($0["ids"] as? [String]) == ["table-7"] })
            var modal = snapshot
            modal["modal"] = true
            modal["sidebar"] = false
            let modalData = try! JSONSerialization.data(withJSONObject: modal)
            modalData.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, modalData.count) }
            assert(split.splitViewItems[0].isCollapsed && !table.isEnabled)
            assert(!addButton.isEnabled, "Modal disables the compact add control")
            var settings = snapshot
            settings["sidebar"] = false
            settings["canToggleSidebar"] = false
            let settingsData = try! JSONSerialization.data(withJSONObject: settings)
            settingsData.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, settingsData.count) }
            assert(window.toolbar!.items.first { $0.itemIdentifier.rawValue == "toggleSidebar" }?.isEnabled == false, "Settings has no explorer to toggle")
            modalData.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, modalData.count) }
            let host = (split.splitViewItems[1].viewController.view as! NSBackgroundExtensionView).contentView!
            assert(host.frame.minX == 0, "Collapsing must not leave a stale extended strip where the sidebar was: \(host.frame)")
            assert(window.toolbar!.items.filter { $0.action != nil }.allSatisfy { !$0.isEnabled })
            data.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, data.count) }
            assert(!split.splitViewItems[0].isCollapsed && table.isEnabled)
            var locked = snapshot
            locked["unlocked"] = false
            locked["connections"] = [] as [[String: String]]
            let lockedData = try! JSONSerialization.data(withJSONObject: locked)
            lockedData.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, lockedData.count) }
            assert(window.toolbar!.items.isEmpty && !window.toolbar!.isVisible, "Locked vault hides all app controls")
            assert(split.splitViewItems[0].isCollapsed)
            data.withUnsafeBytes { shellSync(shell, $0.bindMemory(to: UInt8.self).baseAddress!, data.count) }
            assert(window.toolbar!.isVisible && window.toolbar!.items.contains { $0.itemIdentifier.rawValue == "connection:other" })
            shellDestroy(shell)
            assert(window.contentView === root && renderer.superview === root)
            assert(window.toolbar == nil && window.toolbarStyle == originalStyle)
            assert(window.appearance == nil)
        }
        window.close()
        print("Native shell lifecycle: passed (3 toggles, native selection, tab dragging, accessibility root, scroll coordinates, restoration)")
    }
}
