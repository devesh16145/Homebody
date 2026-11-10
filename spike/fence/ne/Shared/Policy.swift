// Policy.swift — the smallest policy that expresses the Fence: which processes are fenced,
// which hosts they may reach, and whether we are in strict or watch mode.
// Compiled into both the host app and the filter extension.

import Foundation

struct Policy: Codable {
    /// "strict": deny anything not allowed. "watch": allow everything but log it (onboarding).
    var mode: String = "strict"
    /// Case-insensitive substrings matched against the bundle identifier or executable name.
    var fencedProcesses: [String] = ["ollama", "lmstudio", "lm studio", "jan"]
    /// Exact hostnames or wildcard patterns like "*.ollama.ai". Matched against the resolved hostname when known, else the IP.
    var allowHosts: [String] = []
    /// Loopback is the whole point of a local model daemon; keep it on.
    var allowLocalhost: Bool = true

    static let fileName = "policy.json"

    static func load(from directory: URL) -> Policy {
        let url = directory.appendingPathComponent(fileName)
        if let d = try? Data(contentsOf: url), let p = try? JSONDecoder().decode(Policy.self, from: d) {
            return p
        }
        return Policy()
    }

    func save(to directory: URL) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let e = JSONEncoder(); e.outputFormatting = [.prettyPrinted, .sortedKeys]
        try e.encode(self).write(to: directory.appendingPathComponent(Policy.fileName), options: .atomic)
    }

    func isFenced(label: String) -> Bool {
        let l = label.lowercased()
        return fencedProcesses.contains { l.contains($0.lowercased()) }
    }

    func isLocalhost(_ host: String) -> Bool {
        let h = host.lowercased().split(separator: "%").first.map(String.init) ?? host
        return h == "127.0.0.1" || h == "::1" || h == "localhost" || h.hasPrefix("127.") || h == "::ffff:127.0.0.1"
    }

    /// Returns the matching allow pattern, or nil.
    func allowRule(forHost host: String) -> String? {
        let h = host.lowercased()
        for pattern in allowHosts {
            let p = pattern.lowercased()
            if p == h { return pattern }
            if p.hasPrefix("*."), h.hasSuffix(String(p.dropFirst(1))) || h == String(p.dropFirst(2)) { return pattern }
        }
        return nil
    }
}

/// Shared location for policy and receipts: the app group container, so the sandboxed
/// extension (running as root) and the host app (running as the user) see the same files.
enum SharedPaths {
    /// Set at build time by build.sh via -DAPP_GROUP=... is not possible in plain swiftc,
    /// so the group id is read from the bundle's Info.plist key "SAIAppGroup".
    static var appGroup: String {
        (Bundle.main.object(forInfoDictionaryKey: "SAIAppGroup") as? String) ?? "group.com.sandboxai.fence"
    }

    private static var resolved: URL?

    static func container() -> URL {
        if let r = resolved { return r }
        let fallback = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
            .appendingPathComponent("SandboxAI-Fence", isDirectory: true)
        var chosen = fallback
        if let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup), isWritable(url) {
            chosen = url
        }
        resolved = chosen
        return chosen
    }

    /// macOS 15+ silently refuses writes into a group container from a non-sandboxed app whose
    /// group id is not Team-ID-prefixed (the ad-hoc, no-Developer-account case). Probe instead of trusting the URL.
    private static func isWritable(_ dir: URL) -> Bool {
        let probe = dir.appendingPathComponent(".sai-probe-\(getpid())")
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try Data("ok".utf8).write(to: probe, options: .atomic)
            try FileManager.default.removeItem(at: probe)
            return true
        } catch {
            return false
        }
    }

    static func receiptsDirectory() -> URL { container().appendingPathComponent("Receipts", isDirectory: true) }
    static func policyDirectory() -> URL { container() }
}
