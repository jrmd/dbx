import AppKit

// AppKit owns navigation, accessibility, vibrancy and scroll-edge rendering.
// The ABI passes snapshots and queued intents; it never calls Rust reentrantly
// from layout or a delegate (GPUI may already hold its window/entity locks).
#if compiler(>=6.2)
@available(macOS 26.0, *)
private final class HostView: NSView {
    var onLayout: (() -> Void)?
    override func layout() { super.layout(); onLayout?() }
}

@available(macOS 26.0, *)
private final class ContentScrollHost: NSScrollView {
    // GPUI owns wheel/key/accessible scrolling and virtualization. This host
    // only projects its geometry for AppKit's edge effects; never scroll twice.
    override func scrollWheel(with event: NSEvent) {}
}

@available(macOS 26.0, *)
private final class ScrollDocument: NSView {
    override var isFlipped: Bool { true }
}

// A connection tab's select button. Toolbar items have no native reordering,
// so a horizontal drag is tracked here; a plain click still selects the tab.
@available(macOS 26.0, *)
private final class ConnectionTabButton: NSButton {
    var onDrag: ((ConnectionTabButton, NSEvent) -> Void)?
    var onDragEnd: ((ConnectionTabButton, NSEvent?) -> Void)?
    override func mouseDown(with event: NSEvent) {
        guard let window, isEnabled else { return super.mouseDown(with: event) }
        let start = event.locationInWindow
        var dragging = false
        while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            if next.type == .leftMouseUp {
                if dragging { onDragEnd?(self, next) }
                else if bounds.contains(convert(next.locationInWindow, from: nil)) { sendAction(action, to: target) }
                return
            }
            if !dragging, abs(next.locationInWindow.x - start.x) > 4 { dragging = true }
            if dragging { onDrag?(self, next) }
        }
        if dragging { onDragEnd?(self, nil) }
    }
}

// Mirrors GPUI's tag_badge: the tag's name in its colour inside an outline.
// Drawn rather than layer-styled so it follows appearance changes.
@available(macOS 26.0, *)
private final class TagPill: NSView {
    var text = "" { didSet { invalidateIntrinsicContentSize(); needsDisplay = true } }
    var color = NSColor.clear { didSet { needsDisplay = true } }
    private var attributes: [NSAttributedString.Key: Any] {
        [.font: NSFont.systemFont(ofSize: 10, weight: .medium), .foregroundColor: color]
    }
    override var intrinsicContentSize: NSSize {
        let size = (text as NSString).size(withAttributes: attributes)
        return NSSize(width: ceil(size.width) + 12, height: ceil(size.height) + 4)
    }
    override func draw(_ dirtyRect: NSRect) {
        let outline = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5),
            xRadius: (bounds.height - 1) / 2, yRadius: (bounds.height - 1) / 2)
        color.withAlphaComponent(0.12).setFill()
        outline.fill()
        color.setStroke()
        outline.stroke()
        let size = (text as NSString).size(withAttributes: attributes)
        (text as NSString).draw(at: NSPoint(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2),
            withAttributes: attributes)
    }
}

@available(macOS 26.0, *)
private final class ExplorerTable: NSTableView {
    var onReturn: (() -> Void)?
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 36 || event.keyCode == 76 { onReturn?() }
        else { super.keyDown(with: event) }
    }
}

@available(macOS 26.0, *)
private final class NativeShell: NSObject, NSToolbarDelegate, NSTableViewDataSource,
    NSTableViewDelegate, NSSearchFieldDelegate, NSMenuDelegate {
    let renderer: NSView
    weak var window: NSWindow?
    let originalContent: NSView
    let originalController: NSViewController?
    let originalToolbar: NSToolbar?
    let originalTitleVisibility: NSWindow.TitleVisibility
    let originalToolbarStyle: NSWindow.ToolbarStyle
    let originalAppearance: NSAppearance?
    let split = NSSplitViewController()
    let sidebarController = NSViewController()
    let mainController = NSViewController()
    let mainHost = HostView()
    // Blurs the desktop beneath GPUI's translucent content surfaces. A clear
    // Metal layer alone would show the unblurred desktop and windows behind.
    let backdrop = NSVisualEffectView()
    let backgroundExtension = NSBackgroundExtensionView()
    let mainScroll = ContentScrollHost()
    let mainDocument = ScrollDocument()
    let scroll = NSScrollView()
    let table = ExplorerTable()
    let search = NSSearchField()
    let database = NSPopUpButton()
    let schema = NSPopUpButton()
    let explorerActions = NSMenu()
    let sidebarHeader = NSSplitViewItemAccessoryViewController()
    let headerStack = NSStackView()
    let status = NSTextField(labelWithString: "Explorer")
    // AppKit mirrors items between toolbars sharing an identifier, even with
    // autosaving off. Each host needs its own family when toggling/reopening.
    let toolbar = NSToolbar(identifier: "dev.dbx.navigation." + UUID().uuidString)
    var sidebarItem: NSSplitViewItem!
    var model: [String: Any] = [:]
    var rows: [[String: Any]] = []
    var commands: [[String: Any]] = []
    var syncing = false
    var lastGeometry: [Double] = []
    var lastData: Data?
    var modal = false
    var scrollOffset = 0.0
    var scrollMaximum = 0.0
    var observer: NSObjectProtocol?
    // Glass copy of the connection tab being dragged, following the pointer.
    var dragGhost: NSWindow?
    var dragOrigin = NSPoint.zero

    init?(renderer: NSView) {
        guard let window = renderer.window, let content = window.contentView else { return nil }
        self.renderer = renderer
        self.window = window
        originalContent = content
        originalController = window.contentViewController
        originalToolbar = window.toolbar
        originalTitleVisibility = window.titleVisibility
        originalToolbarStyle = window.toolbarStyle
        originalAppearance = window.appearance
        super.init()

        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        table.headerView = nil
        table.backgroundColor = .clear
        table.style = .sourceList
        table.rowHeight = 28
        table.intercellSpacing = NSSize(width: 0, height: 2)
        table.allowsMultipleSelection = true
        table.allowsEmptySelection = true
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("name"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        table.delegate = self
        table.dataSource = self
        table.target = self
        table.action = #selector(openRow)
        table.onReturn = { [weak self] in self?.openRow() }
        table.setAccessibilityLabel("Database explorer")
        scroll.documentView = table
        let menu = NSMenu()
        menu.delegate = self
        table.menu = menu
        sidebarController.view = scroll

        search.placeholderString = "Search tables"
        search.setAccessibilityLabel("Search tables")
        search.delegate = self
        search.sendsSearchStringImmediately = true
        database.target = self
        database.action = #selector(selectDatabase)
        database.setAccessibilityLabel("Database")
        schema.target = self
        schema.action = #selector(selectSchema)
        schema.setAccessibilityLabel("Schema filter")
        status.font = .systemFont(ofSize: 12, weight: .semibold)
        status.textColor = .secondaryLabelColor
        let refresh = button("Refresh explorer", symbol: "arrow.clockwise", command: "refreshExplorer")
        let create = button("New table", symbol: "plus", command: "newTable")
        let more = button("Explorer actions", symbol: "ellipsis", command: "explorerActions")
        let title = NSStackView(views: [status, refresh, create, more])
        title.orientation = .horizontal
        title.spacing = 6
        status.setContentHuggingPriority(.defaultLow, for: .horizontal)
        status.lineBreakMode = .byTruncatingTail
        search.heightAnchor.constraint(equalToConstant: 32).isActive = true
        headerStack.orientation = .vertical
        headerStack.alignment = .leading
        headerStack.spacing = 8
        [title, database, schema, search].forEach { view in
            headerStack.addArrangedSubview(view)
            view.translatesAutoresizingMaskIntoConstraints = false
            view.widthAnchor.constraint(equalTo: headerStack.widthAnchor, constant: -24).isActive = true
        }
        headerStack.edgeInsets = NSEdgeInsets(top: 8, left: 12, bottom: 10, right: 12)
        sidebarHeader.view = headerStack
        // Automatic scroll-edge styling works on macOS 26.0 and later.
        sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarController)
        sidebarItem.minimumThickness = 240
        sidebarItem.maximumThickness = 440
        sidebarItem.canCollapse = true
        sidebarItem.addTopAlignedAccessoryViewController(sidebarHeader)
        // Let AppKit extend the content's colours beneath both navigation
        // regions. A raw transparent toolbar would expose an unmatched desktop
        // strip; an opaque full-window renderer would split their backdrops.
        backgroundExtension.contentView = mainHost
        mainController.view = backgroundExtension
        let contentItem = NSSplitViewItem(viewController: mainController)
        contentItem.automaticallyAdjustsSafeAreaInsets = true
        split.addSplitViewItem(sidebarItem)
        split.addSplitViewItem(contentItem)
        split.splitView.isVertical = true
        split.splitView.dividerStyle = .thin
        split.view.frame = content.frame
        mainScroll.setAccessibilityRole(.group)
        mainScroll.setAccessibilityLabel("Workspace")
        mainScroll.drawsBackground = false
        mainScroll.borderType = .noBorder
        mainScroll.hasVerticalScroller = false
        mainScroll.hasHorizontalScroller = false
        mainScroll.verticalScrollElasticity = .none
        mainScroll.horizontalScrollElasticity = .none
        mainScroll.autoresizingMask = [.width, .height]
        mainScroll.documentView = mainDocument
        backdrop.material = .underWindowBackground
        backdrop.blendingMode = .behindWindow
        backdrop.state = .active
        backdrop.frame = mainHost.bounds
        backdrop.autoresizingMask = [.width, .height]
        mainHost.addSubview(backdrop)
        mainHost.addSubview(mainScroll)
        originalContent.autoresizingMask = []
        renderer.autoresizingMask = [.width, .height]
        mainHost.onLayout = { [weak self] in self?.layoutRenderer() }
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        toolbar.autosavesConfiguration = false
        window.contentViewController = split
        // Preserve GPUI's original content root, where AccessKit is attached.
        mainDocument.addSubview(originalContent)
        window.titleVisibility = .hidden
        window.toolbarStyle = .unified
        window.toolbar = toolbar
        toolbar.isVisible = false
        split.splitView.setPosition(260, ofDividerAt: 0)
        observer = NotificationCenter.default.addObserver(forName: NSSplitView.didResizeSubviewsNotification,
            object: split.splitView, queue: .main) { [weak self] _ in self?.layoutRenderer() }
        layoutRenderer()
        window.makeFirstResponder(renderer)
        layoutRenderer()
    }

    func detach() {
        if let observer { NotificationCenter.default.removeObserver(observer) }
        mainHost.onLayout = nil
        guard let window else { return }
        originalContent.removeFromSuperview()
        window.toolbar = originalToolbar
        window.titleVisibility = originalTitleVisibility
        window.toolbarStyle = originalToolbarStyle
        window.appearance = originalAppearance
        window.contentViewController = originalController
        window.contentView = originalContent
        originalContent.autoresizingMask = [.width, .height]
        renderer.autoresizingMask = [.width, .height]
        renderer.frame = originalContent.bounds
        window.makeFirstResponder(renderer)
    }

    func layoutRenderer() {
        guard let window, let root = window.contentView else { return }
        // GPUI's platform input, IME and accessibility use window coordinates.
        // Keep its logical canvas equal to the full contentView and its origin
        // at window zero. The main split clips it to the unobscured pane; Rust
        // reserves the native sidebar/titlebar space in that same canvas. No
        // patches to GPUI's Metal renderer or synthesized mouse events needed.
        mainScroll.frame = mainHost.bounds
        let maximum = max(0, scrollMaximum)
        let offset = min(maximum, max(0, scrollOffset))
        mainDocument.setFrameSize(NSSize(width: mainHost.bounds.width, height: mainHost.bounds.height + maximum))
        let desiredOrigin = NSPoint(x: 0, y: offset - mainScroll.contentInsets.top)
        if mainScroll.contentView.bounds.origin != desiredOrigin {
            mainScroll.contentView.scroll(to: desiredOrigin)
            mainScroll.reflectScrolledClipView(mainScroll.contentView)
        }
        let frame = mainDocument.convert(root.bounds, from: root)
        if originalContent.frame != frame { originalContent.frame = frame }
        if renderer.frame != originalContent.bounds { renderer.frame = originalContent.bounds }
        let visible = root.convert(mainHost.bounds, from: mainHost)
        let top = max(0, root.bounds.height - window.contentLayoutRect.maxY)
        // With background extension enabled, the content's frame reaches
        // beneath the floating sidebar. Reserve the sidebar's actual split
        // frame, not the extended content origin, in GPUI's logical canvas.
        let sidebarFrame = split.splitView.arrangedSubviews.first.map {
            root.convert($0.bounds, from: $0)
        } ?? .zero
        let left = sidebarItem.isCollapsed ? visible.minX : max(visible.minX, sidebarFrame.maxX + split.splitView.dividerThickness)
        let geometry = [Double(left), Double(top)]
        if geometry != lastGeometry {
            lastGeometry = geometry
            commands.append(["kind": "geometry", "left": geometry[0], "top": geometry[1], "collapsed": sidebarItem.isCollapsed])
        }
    }

    func send(_ kind: String, _ extra: [String: Any] = [:]) {
        guard !syncing, !modal, model["unlocked"] as? Bool == true else { return }
        var command = extra
        command["kind"] = kind
        command["session"] = model["session"]
        commands.append(command)
        if !["search", "selection", "schema", "database"].contains(kind) {
            window?.makeFirstResponder(renderer)
            // Focusing a descendant can ask NSClipView to reveal its bounds.
            // Re-pin the projected canvas after that native focus adjustment.
            layoutRenderer()
        }
    }
    func button(_ label: String, symbol: String, command: String) -> NSButton {
        let result = NSButton(image: NSImage(systemSymbolName: symbol, accessibilityDescription: label)!, target: self, action: #selector(controlAction(_:)))
        result.bezelStyle = .accessoryBarAction
        result.isBordered = false
        result.imageScaling = .scaleProportionallyDown
        result.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        result.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            result.widthAnchor.constraint(equalToConstant: 32),
            result.heightAnchor.constraint(equalToConstant: 32)
        ])
        result.identifier = NSUserInterfaceItemIdentifier(command)
        result.toolTip = label
        result.setAccessibilityLabel(label)
        return result
    }
    @objc func controlAction(_ sender: NSControl) {
        guard let id = sender.identifier?.rawValue else { return }
        if id == "explorerActions" {
            explorerActions.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.minY - 4), in: sender)
        } else { send(id) }
    }
    @objc func toolbarAction(_ sender: NSToolbarItem) { send(sender.itemIdentifier.rawValue) }
    @objc func selectConnection(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue else { return }
        send("connection", ["id": id])
    }
    func connectionTabViews() -> [(id: String, view: NSView)] {
        toolbar.items.compactMap { item in
            guard item.itemIdentifier.rawValue.hasPrefix("connection:"), let view = item.view else { return nil }
            return (String(item.itemIdentifier.rawValue.dropFirst("connection:".count)), view)
        }
    }
    func dragConnection(_ button: NSButton, _ event: NSEvent) {
        guard let window, let tab = button.superview else { return }
        if dragGhost == nil {
            // Lift a glass copy of the tab; the original stays dimmed in place.
            let frame = tab.convert(tab.bounds, to: nil)
            let snapshot = NSImage(size: tab.bounds.size)
            if let rep = tab.bitmapImageRepForCachingDisplay(in: tab.bounds) {
                tab.cacheDisplay(in: tab.bounds, to: rep)
                snapshot.addRepresentation(rep)
            }
            let glass = NSGlassEffectView(frame: NSRect(origin: .zero, size: frame.size))
            glass.cornerRadius = frame.height / 2
            glass.contentView = NSImageView(image: snapshot)
            let ghost = NSPanel(contentRect: window.convertToScreen(frame), styleMask: [.borderless, .nonactivatingPanel],
                backing: .buffered, defer: false)
            ghost.isOpaque = false
            ghost.backgroundColor = .clear
            ghost.hasShadow = true
            ghost.ignoresMouseEvents = true
            ghost.contentView = glass
            window.addChildWindow(ghost, ordered: .above)
            dragGhost = ghost
            dragOrigin = event.locationInWindow
            tab.alphaValue = 0.35
        }
        guard let ghost = dragGhost else { return }
        let frame = window.convertToScreen(tab.convert(tab.bounds, to: nil))
        ghost.setFrameOrigin(NSPoint(x: frame.minX + event.locationInWindow.x - dragOrigin.x, y: frame.minY))
    }
    func dropConnection(_ button: NSButton, _ event: NSEvent?) {
        button.superview?.alphaValue = 1
        if let ghost = dragGhost { window?.removeChildWindow(ghost); ghost.orderOut(nil) }
        dragGhost = nil
        guard let event, let id = button.identifier?.rawValue else { return }
        // Land in the slot of the tab whose centre is nearest the pointer.
        let x = event.locationInWindow.x
        let target = connectionTabViews().min { lhs, rhs in
            abs(lhs.view.convert(lhs.view.bounds, to: nil).midX - x) < abs(rhs.view.convert(rhs.view.bounds, to: nil).midX - x)
        }
        if let target, target.id != id { send("moveConnection", ["id": id, "target": target.id]) }
    }
    @objc func closeConnectionTab(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue else { return }
        send("closeConnection", ["id": id])
    }
    @objc func selectDatabase() { send("database", ["value": database.titleOfSelectedItem ?? ""]) }
    @objc func selectSchema() { send("schema", ["value": schema.indexOfSelectedItem == 0 ? "" : schema.titleOfSelectedItem ?? ""]) }
    func controlTextDidChange(_ obj: Notification) { send("search", ["value": search.stringValue]) }
    @objc func openRow() {
        guard table.selectedRow >= 0, table.selectedRow < rows.count else { return }
        let selected = table.selectedRowIndexes.compactMap { rows[$0]["id"] as? String }
        send("selection", ["ids": selected, "open": selected.count == 1])
    }
    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !syncing else { return }
        let selected = table.selectedRowIndexes.compactMap { rows[$0]["id"] as? String }
        send("selection", ["ids": selected, "open": false])
    }
    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = NSUserInterfaceItemIdentifier("explorerCell")
        let cell = (tableView.makeView(withIdentifier: identifier, owner: self) as? NSTableCellView) ?? NSTableCellView()
        if cell.textField == nil {
            cell.identifier = identifier
            let label = NSTextField(labelWithString: "")
            label.lineBreakMode = .byTruncatingMiddle
            label.translatesAutoresizingMaskIntoConstraints = false
            let image = NSImageView()
            image.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(image)
            cell.addSubview(label)
            cell.textField = label
            cell.imageView = image
            NSLayoutConstraint.activate([
                image.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
                image.widthAnchor.constraint(equalToConstant: 16), image.heightAnchor.constraint(equalToConstant: 16),
                image.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
                label.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 7),
                label.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
                label.centerYAnchor.constraint(equalTo: cell.centerYAnchor)
            ])
        }
        let item = rows[row]
        cell.textField?.stringValue = item["label"] as? String ?? ""
        cell.imageView?.image = NSImage(systemSymbolName: item["symbol"] as? String ?? "tablecells", accessibilityDescription: nil)
        cell.toolTip = cell.textField?.stringValue
        return cell
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { toolbarDefaultItemIdentifiers(toolbar) }
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        guard model["unlocked"] as? Bool == true else { return [] }
        let connections = (model["connections"] as? [[String: Any]] ?? []).compactMap { session in
            (session["id"] as? String).map { NSToolbarItem.Identifier("connection:" + $0) }
        }
        // Separate each connection's glass capsule rather than merging the
        // entire tab strip and Add button into one native toolbar group.
        let tabs = connections.flatMap { [$0, NSToolbarItem.Identifier.space] }
        return [NSToolbarItem.Identifier("toggleSidebar"), .sidebarTrackingSeparator]
            + tabs
            + [NSToolbarItem.Identifier("newConnection"), .flexibleSpace,
               NSToolbarItem.Identifier("data"), NSToolbarItem.Identifier("structure"),
               NSToolbarItem.Identifier("newQuery"), NSToolbarItem.Identifier("refresh"), .space,
               NSToolbarItem.Identifier("lock"), NSToolbarItem.Identifier("settings")]
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier id: NSToolbarItem.Identifier, willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        if id == .sidebarTrackingSeparator {
            return NSTrackingSeparatorToolbarItem(identifier: id, splitView: split.splitView, dividerIndex: 0)
        }
        let item = NSToolbarItem(itemIdentifier: id)
        if id.rawValue.hasPrefix("connection:") {
            let sessionID = String(id.rawValue.dropFirst("connection:".count))
            let select = ConnectionTabButton(title: "", target: self, action: #selector(selectConnection(_:)))
            select.onDrag = { [weak self] button, event in self?.dragConnection(button, event) }
            select.onDragEnd = { [weak self] button, event in self?.dropConnection(button, event) }
            select.identifier = NSUserInterfaceItemIdentifier(sessionID)
            select.setButtonType(.toggle)
            select.isBordered = false
            select.imagePosition = .imageLeading
            let close = NSButton(image: NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close connection")!, target: self, action: #selector(closeConnectionTab(_:)))
            close.identifier = NSUserInterfaceItemIdentifier(sessionID)
            close.isBordered = false
            close.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 10, weight: .medium)
            close.translatesAutoresizingMaskIntoConstraints = false
            close.widthAnchor.constraint(equalToConstant: 24).isActive = true
            close.heightAnchor.constraint(equalToConstant: 28).isActive = true
            let tag = TagPill()
            tag.setAccessibilityElement(false)
            tag.isHidden = true
            let tabs = NSStackView(views: [select, tag, close])
            tabs.orientation = .horizontal
            tabs.spacing = 6
            tabs.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 4)
            item.view = tabs
            item.isBordered = true
            item.visibilityPriority = .high
            item.autovalidates = false
            return item
        }
        let labels: [String: (String, String)] = [
            "toggleSidebar": ("Toggle explorer", "sidebar.left"), "newConnection": ("New connection", "plus"),
            "data": ("Data", "tablecells"), "structure": ("Structure", "list.bullet.rectangle"),
            "newQuery": ("New query", "square.and.pencil"), "refresh": ("Refresh", "arrow.clockwise"),
            "lock": ("Lock vault", "lock"), "settings": ("Settings", "gearshape")]
        guard let (label, symbol) = labels[id.rawValue] else { return nil }
        item.label = label
        item.toolTip = label
        item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
        item.target = self
        item.action = #selector(toolbarAction(_:))
        item.autovalidates = false
        return item
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let row = table.clickedRow >= 0 ? table.clickedRow : table.selectedRow
        guard row >= 0, row < rows.count else { return }
        if !table.isRowSelected(row) { table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        let ids = table.selectedRowIndexes.compactMap { rows[$0]["id"] as? String }
        for entry in model["tableMenu"] as? [[String: Any]] ?? [] {
            let command = entry["command"] as? String ?? ""
            if ids.count > 1 && !["OpenData", "OpenStructure", "CopyName", "Export", "Truncate", "Drop"].contains(command) && entry["separator"] as? Bool != true { continue }
            if entry["separator"] as? Bool == true { menu.addItem(.separator()); continue }
            let item = NSMenuItem(title: entry["label"] as? String ?? "", action: #selector(tableMenuAction(_:)), keyEquivalent: "")
            item.target = self
            item.isEnabled = !modal && (entry["enabled"] as? Bool ?? false)
            if ["Rename", "Duplicate", "Truncate", "Drop", "Import"].contains(command),
               table.selectedRowIndexes.contains(where: { rows[$0]["writable"] as? Bool != true }) { item.isEnabled = false }
            item.representedObject = ["command": entry["command"] ?? "", "ids": ids] as [String: Any]
            menu.addItem(item)
        }
        menu.autoenablesItems = false
    }
    @objc func tableMenuAction(_ sender: NSMenuItem) { send("tableCommand", sender.representedObject as? [String: Any] ?? [:]) }
    @objc func explorerMenuAction(_ sender: NSMenuItem) { send(sender.representedObject as? String ?? "") }

    func sync(_ data: Data) {
        guard data != lastData, let next = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
        lastData = data
        syncing = true
        defer { syncing = false }
        model = next
        let wasModal = modal
        modal = next["modal"] as? Bool ?? false
        if modal && !wasModal { window?.makeFirstResponder(renderer) }
        let dark = next["dark"] as? Bool ?? false
        // Set only when changed: a window appearance notification can make GPUI
        // re-enter material resolution on its next frame.
        let appearance: NSAppearance.Name? = next["systemAppearance"] as? Bool == true ? nil : (dark ? .darkAqua : .aqua)
        split.view.appearance = appearance.flatMap { NSAppearance(named: $0) }
        if window?.appearance?.name != appearance {
            window?.appearance = appearance.flatMap { NSAppearance(named: $0) }
        }
        let visible = next["unlocked"] as? Bool == true && (next["sidebar"] as? Bool ?? false)
        let collapseChanged = sidebarItem.isCollapsed == visible
        if collapseChanged { sidebarItem.isCollapsed = !visible }
        let nextRows = next["rows"] as? [[String: Any]] ?? []
        if !NSArray(array: rows).isEqual(to: nextRows) {
            rows = nextRows
            table.reloadData()
        }
        let selected = Set(next["selected"] as? [String] ?? [])
        let indexes = IndexSet(rows.indices.filter { selected.contains(rows[$0]["id"] as? String ?? "") })
        if table.selectedRowIndexes != indexes { table.selectRowIndexes(indexes, byExtendingSelection: false) }
        status.stringValue = next["status"] as? String ?? "Explorer"
        if search.currentEditor() == nil { search.stringValue = next["search"] as? String ?? "" }
        syncPopup(database, items: next["databases"] as? [String] ?? [], selected: next["database"] as? String)
        syncPopup(schema, items: next["schemas"] as? [String] ?? [], selected: next["schema"] as? String)
        database.isHidden = database.numberOfItems < 2
        schema.isHidden = schema.numberOfItems < 3
        let sessions = next["connections"] as? [[String: Any]] ?? []
        let connected = next["connected"] as? Bool ?? false
        let unlocked = next["unlocked"] as? Bool ?? false
        let identifiers = toolbarDefaultItemIdentifiers(toolbar)
        if toolbar.items.map({ $0.itemIdentifier }) != identifiers {
            while !toolbar.items.isEmpty { toolbar.removeItem(at: toolbar.items.count - 1) }
            for (index, identifier) in identifiers.enumerated() { toolbar.insertItem(withItemIdentifier: identifier, at: index) }
        }
        toolbar.isVisible = unlocked
        let canToggleSidebar = next["canToggleSidebar"] as? Bool ?? false
        for item in toolbar.items {
            item.isEnabled = !modal && (item.itemIdentifier.rawValue == "settings" || (unlocked && (!["data", "structure", "newQuery", "refresh", "toggleSidebar"].contains(item.itemIdentifier.rawValue) || connected)))
            if item.itemIdentifier.rawValue == "toggleSidebar" { item.isEnabled = item.isEnabled && canToggleSidebar }
            if item.itemIdentifier.rawValue.hasPrefix("connection:"),
               let stack = item.view as? NSStackView,
               let select = stack.arrangedSubviews.first as? NSButton,
               let close = stack.arrangedSubviews.last as? NSButton,
               let tag = stack.arrangedSubviews.first(where: { $0 is TagPill }) as? TagPill,
               let session = sessions.first(where: { $0["id"] as? String == select.identifier?.rawValue }) {
                let label = session["label"] as? String ?? "Connection"
                let active = session["id"] as? String == next["session"] as? String
                select.state = active ? .on : .off
                // Explicit attributes: a borderless toggle otherwise tints its
                // whole "on" title with the accent colour.
                let title = NSMutableAttributedString(string: session["name"] as? String ?? label, attributes: [
                    .font: NSFont.systemFont(ofSize: 13, weight: active ? .semibold : .regular),
                    .foregroundColor: active ? NSColor.labelColor : NSColor.secondaryLabelColor])
                if session["busy"] as? Bool == true {
                    title.append(NSAttributedString(string: " · working", attributes: [
                        .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.tertiaryLabelColor]))
                }
                select.attributedTitle = title
                select.attributedAlternateTitle = title
                select.image = NSImage(systemSymbolName: "externaldrive", accessibilityDescription: nil)
                select.contentTintColor = active ? .labelColor : .secondaryLabelColor
                if let info = session["tag"] as? [String: Any], let name = info["name"] as? String,
                   let rgb = info["color"] as? Int {
                    tag.text = name
                    tag.color = NSColor(srgbRed: CGFloat((rgb >> 16) & 0xff) / 255,
                        green: CGFloat((rgb >> 8) & 0xff) / 255, blue: CGFloat(rgb & 0xff) / 255, alpha: 1)
                    tag.isHidden = false
                } else {
                    tag.isHidden = true
                }
                select.setAccessibilityLabel(label)
                select.setAccessibilityRole(.radioButton)
                select.isEnabled = !modal
                close.isEnabled = !modal
                close.setAccessibilityLabel("Close connection " + label)
                close.toolTip = "Close connection " + label
                item.label = label
                item.toolTip = label
            }
        }
        search.isEnabled = !modal
        database.isEnabled = !modal
        schema.isEnabled = !modal
        table.isEnabled = !modal
        explorerActions.removeAllItems()
        explorerActions.autoenablesItems = false
        for entry in next["explorerMenu"] as? [[String: Any]] ?? [] {
            let item = NSMenuItem(title: entry["label"] as? String ?? "", action: #selector(explorerMenuAction(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = entry["command"]
            item.isEnabled = !modal && (entry["enabled"] as? Bool ?? true)
            explorerActions.addItem(item)
        }
        if let title = headerStack.arrangedSubviews.first as? NSStackView {
            title.arrangedSubviews.compactMap { $0 as? NSControl }.forEach { control in
                control.isEnabled = !modal && (control.identifier?.rawValue != "newTable" || next["canCreateTable"] as? Bool == true)
            }
        }
        split.view.needsLayout = true
        if collapseChanged {
            // Collapsing resizes the split lazily. Settle it now and once more
            // after AppKit's pass, or GPUI keeps the stale sidebar inset until
            // the next window resize. The background extension view does not
            // re-place its content when only its safe area changes, leaving a
            // stale mirrored strip (and GPUI inset) where the sidebar was.
            split.view.layoutSubtreeIfNeeded()
            backgroundExtension.needsLayout = true
            backgroundExtension.layoutSubtreeIfNeeded()
            DispatchQueue.main.async { [weak self] in
                self?.split.view.layoutSubtreeIfNeeded()
                self?.backgroundExtension.needsLayout = true
                self?.backgroundExtension.layoutSubtreeIfNeeded()
                self?.layoutRenderer()
            }
        }
        layoutRenderer()
    }
    func syncPopup(_ popup: NSPopUpButton, items: [String], selected: String?) {
        if popup.itemTitles != items { popup.removeAllItems(); popup.addItems(withTitles: items) }
        if let selected { popup.selectItem(withTitle: selected) }
    }
}
#endif

@_cdecl("dbx_native_shell_supported")
func shellSupported() -> Bool {
#if compiler(>=6.2)
    if #available(macOS 26.0, *) { return true }
#endif
    return false
}
@_cdecl("dbx_native_shell_create")
func shellCreate(_ view: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer? {
#if compiler(>=6.2)
    if #available(macOS 26.0, *), let shell = NativeShell(renderer: Unmanaged<NSView>.fromOpaque(view).takeUnretainedValue()) {
        return Unmanaged.passRetained(shell).toOpaque()
    }
#endif
    return nil
}
@_cdecl("dbx_native_shell_destroy")
func shellDestroy(_ pointer: UnsafeMutableRawPointer) {
#if compiler(>=6.2)
    if #available(macOS 26.0, *) {
        let shell = Unmanaged<NativeShell>.fromOpaque(pointer).takeRetainedValue()
        shell.detach()
    }
#endif
}
@_cdecl("dbx_native_shell_sync")
func shellSync(_ pointer: UnsafeMutableRawPointer, _ bytes: UnsafePointer<UInt8>, _ count: Int) {
#if compiler(>=6.2)
    if #available(macOS 26.0, *) { Unmanaged<NativeShell>.fromOpaque(pointer).takeUnretainedValue().sync(Data(bytes: bytes, count: count)) }
#endif
}
@_cdecl("dbx_native_shell_drain")
func shellDrain(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutablePointer<CChar>? {
#if compiler(>=6.2)
    if #available(macOS 26.0, *) {
        let shell = Unmanaged<NativeShell>.fromOpaque(pointer).takeUnretainedValue()
        // The sidebar collapse animates and posts no final resize. Re-measure
        // on every poll (deduplicated by lastGeometry) so GPUI's inset always
        // converges on the settled layout.
        shell.layoutRenderer()
        guard !shell.commands.isEmpty, let data = try? JSONSerialization.data(withJSONObject: shell.commands), let text = String(data: data, encoding: .utf8) else { return nil }
        shell.commands.removeAll()
        return strdup(text)
    }
#endif
    return nil
}

@_cdecl("dbx_native_shell_focus_explorer")
func shellFocusExplorer(_ pointer: UnsafeMutableRawPointer) {
#if compiler(>=6.2)
    if #available(macOS 26.0, *) {
        let shell = Unmanaged<NativeShell>.fromOpaque(pointer).takeUnretainedValue()
        shell.sidebarItem.isCollapsed = false
        shell.window?.makeFirstResponder(shell.table)
    }
#endif
}

@_cdecl("dbx_native_shell_scroll")
func shellScroll(_ pointer: UnsafeMutableRawPointer, _ offset: Double, _ maximum: Double) {
#if compiler(>=6.2)
    if #available(macOS 26.0, *) {
        let shell = Unmanaged<NativeShell>.fromOpaque(pointer).takeUnretainedValue()
        if shell.scrollOffset != offset || shell.scrollMaximum != maximum {
            shell.scrollOffset = offset
            shell.scrollMaximum = maximum
            shell.layoutRenderer()
        }
    }
#endif
}
