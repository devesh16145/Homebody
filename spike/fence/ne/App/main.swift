// Fence host app — a menu-bar-only app that installs the system extension, enables the
// content filter, shows status, and gives access to the policy and receipts.
// This is the "Glance" surface at spike fidelity: a status item and a menu, nothing more.

import AppKit
import NetworkExtension
import SystemExtensions

let extensionID = "com.sandboxai.fence.extension"

final class AppDelegate: NSObject, NSApplicationDelegate, OSSystemExtensionRequestDelegate {
    private var statusItem: NSStatusItem!
    private let statusLine = NSMenuItem(title: "Checking…", action: nil, keyEquivalent: "")
    private let receipts = ReceiptLog(directory: SharedPaths.receiptsDirectory())

    func applicationDidFinishLaunching(_ notification: Notification) {
        ensureDefaultPolicy()
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.title = "⬤"
        statusItem.menu = buildMenu()
        refreshStatus()
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in self?.refreshStatus() }
    }

    // MARK: menu

    private func buildMenu() -> NSMenu {
        let m = NSMenu()
        statusLine.isEnabled = false
        m.addItem(statusLine)
        m.addItem(.separator())
        m.addItem(item("Install Fence extension", #selector(installExtension)))
        m.addItem(item("Turn Private mode on", #selector(enableFilter)))
        m.addItem(item("Turn Private mode off", #selector(disableFilter)))
        m.addItem(.separator())
        m.addItem(item("Show recent activity", #selector(showActivity)))
        m.addItem(item("Verify receipts", #selector(verifyReceipts)))
        m.addItem(item("Open policy file", #selector(openPolicy)))
        m.addItem(item("Open receipts folder", #selector(openReceipts)))
        m.addItem(.separator())
        m.addItem(item("Quit", #selector(quit), key: "q"))
        return m
    }

    private func item(_ title: String, _ sel: Selector, key: String = "") -> NSMenuItem {
        let i = NSMenuItem(title: title, action: sel, keyEquivalent: key)
        i.target = self
        return i
    }

    private func refreshStatus() {
        NEFilterManager.shared().loadFromPreferences { [weak self] _ in
            DispatchQueue.main.async {
                let on = NEFilterManager.shared().isEnabled
                self?.statusItem.button?.title = on ? "🟢" : "⚪️"
                let count = (try? self?.receipts.readAll().count) ?? 0
                self?.statusLine.title = on ? "Private mode is on · \(count) receipts" : "Private mode is off"
            }
        }
    }

    // MARK: system extension

    @objc private func installExtension() {
        let req = OSSystemExtensionRequest.activationRequest(forExtensionWithIdentifier: extensionID, queue: .main)
        req.delegate = self
        OSSystemExtensionManager.shared.submitRequest(req)
        notify("Installing the Fence extension", "macOS will ask you to allow it in System Settings → General → Login Items & Extensions.")
    }

    func request(_ request: OSSystemExtensionRequest, actionForReplacingExtension existing: OSSystemExtensionProperties,
                 withExtension ext: OSSystemExtensionProperties) -> OSSystemExtensionRequest.ReplacementAction { .replace }

    func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        notify("Approval needed", "Allow the Sandbox-ai Fence extension in System Settings, then come back.")
    }

    func request(_ request: OSSystemExtensionRequest, didFinishWithResult result: OSSystemExtensionRequest.Result) {
        notify("Extension installed", result == .completed ? "Now turn Private mode on." : "Will complete after reboot.")
    }

    func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        notify("Extension install failed", error.localizedDescription)
    }

    // MARK: filter configuration

    @objc private func enableFilter() { setFilter(enabled: true) }
    @objc private func disableFilter() { setFilter(enabled: false) }

    private func setFilter(enabled: Bool) {
        let mgr = NEFilterManager.shared()
        mgr.loadFromPreferences { [weak self] err in
            if let err { self?.notify("Could not load filter settings", err.localizedDescription); return }
            if mgr.providerConfiguration == nil {
                let cfg = NEFilterProviderConfiguration()
                cfg.filterSockets = true
                cfg.filterPackets = false
                mgr.providerConfiguration = cfg
            }
            mgr.localizedDescription = "Sandbox-ai Fence"
            mgr.isEnabled = enabled
            mgr.saveToPreferences { err in
                DispatchQueue.main.async {
                    if let err { self?.notify("Could not save filter settings", err.localizedDescription) }
                    else { self?.notify(enabled ? "Private mode is on" : "Private mode is off",
                                        enabled ? "Fenced AI apps can only reach your computer and trusted destinations." : "") }
                    self?.refreshStatus()
                }
            }
        }
    }

    // MARK: receipts and policy

    @objc private func showActivity() {
        let all = (try? receipts.readAll()) ?? []
        let lines = all.suffix(15).map { r -> String in
            let b = r.body
            let verb = b.decision.hasSuffix("allow") ? "Allowed" : "Blocked"
            return "\(verb)  \(b.process) → \(b.dstHost ?? b.dstIP):\(b.dstPort)  [\(b.rule)]"
        }
        alert("Recent activity", lines.isEmpty ? "Nothing recorded yet." : lines.joined(separator: "\n"))
    }

    @objc private func verifyReceipts() {
        do { let n = try receipts.verify(); alert("Receipts verified", "\(n) receipts, chain intact, all signatures valid.") }
        catch { alert("Verification failed", "\(error)") }
    }

    @objc private func openPolicy() {
        ensureDefaultPolicy()
        NSWorkspace.shared.open(SharedPaths.policyDirectory().appendingPathComponent(Policy.fileName))
    }

    @objc private func openReceipts() {
        try? FileManager.default.createDirectory(at: SharedPaths.receiptsDirectory(), withIntermediateDirectories: true)
        NSWorkspace.shared.open(SharedPaths.receiptsDirectory())
    }

    @objc private func quit() { NSApp.terminate(nil) }

    private func ensureDefaultPolicy() {
        let dir = SharedPaths.policyDirectory()
        let url = dir.appendingPathComponent(Policy.fileName)
        NSLog("Fence: shared container = %@", dir.path)
        if !FileManager.default.fileExists(atPath: url.path) {
            do { try Policy().save(to: dir); NSLog("Fence: wrote default policy to %@", url.path) }
            catch { NSLog("Fence: policy save failed: %@", "\(error)") }
        }
    }

    // MARK: UI helpers

    private func alert(_ title: String, _ text: String) {
        let a = NSAlert(); a.messageText = title; a.informativeText = text; a.alertStyle = .informational
        NSApp.activate(ignoringOtherApps: true); a.runModal()
    }

    private func notify(_ title: String, _ text: String) {
        // Spike: alerts instead of UNUserNotification to avoid another entitlement dance.
        DispatchQueue.main.async { self.alert(title, text) }
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
