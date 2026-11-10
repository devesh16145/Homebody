// FilterDataProvider.swift — the Fence as a macOS Network Extension content filter.
//
// Every new outbound socket flow on the machine is presented here. We attribute it to a
// process, consult the policy, return allow or drop, and write a signed receipt for fenced
// processes. Non-fenced processes are allowed without logging so the log stays about the AI.
//
// Requires: com.apple.developer.networking.networkextension = content-filter-provider-systemextension
// and an app group shared with the host app. See ../README.md for the signing prerequisites.

import Foundation
import NetworkExtension
import Network
import Security
import os.log

final class FilterDataProvider: NEFilterDataProvider {

    private let log = OSLog(subsystem: "com.sandboxai.fence", category: "filter")
    private let queue = DispatchQueue(label: "com.sandboxai.fence.receipts")
    private var policy = Policy()
    private var receipts: ReceiptLog!
    private var policyWatcher: DispatchSourceFileSystemObject?

    // MARK: lifecycle

    override func startFilter(completionHandler: @escaping (Error?) -> Void) {
        receipts = ReceiptLog(directory: SharedPaths.receiptsDirectory())
        do { try receipts.initKey() } catch {
            os_log("could not create signing key: %{public}@", log: log, type: .error, "\(error)")
        }
        reloadPolicy()
        watchPolicy()

        // Ask the system to send us every outbound flow, any protocol, any destination.
        let anyOutbound = NENetworkRule(remoteNetwork: nil, remotePrefix: 0,
                                        localNetwork: nil, localPrefix: 0,
                                        protocol: .any, direction: .outbound)
        let rule = NEFilterRule(networkRule: anyOutbound, action: .filterData)
        let settings = NEFilterSettings(rules: [rule], defaultAction: .allow)
        apply(settings) { error in
            if let error { os_log("apply settings failed: %{public}@", log: self.log, type: .error, "\(error)") }
            else { os_log("fence active, mode=%{public}@", log: self.log, type: .info, self.policy.mode) }
            completionHandler(error)
        }
    }

    override func stopFilter(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        policyWatcher?.cancel()
        os_log("fence stopped: %{public}d", log: log, type: .info, reason.rawValue)
        completionHandler()
    }

    // MARK: decisions

    override func handleNewFlow(_ flow: NEFilterFlow) -> NEFilterNewFlowVerdict {
        guard let socket = flow as? NEFilterSocketFlow else { return .allow() }

        let (label, pid) = attribute(flow)
        guard policy.isFenced(label: label) else { return .allow() }

        let (host, port) = remote(of: socket)
        let hostname = socket.remoteHostname ?? host
        let proto: String = socket.socketProtocol == IPPROTO_UDP ? "udp" : (socket.socketProtocol == IPPROTO_TCP ? "tcp" : "other")

        if policy.allowLocalhost && policy.isLocalhost(host) {
            return .allow()   // the model daemon serving local apps; not worth a receipt
        }

        let decision: String
        let rule: String
        if let pattern = policy.allowRule(forHost: hostname) ?? policy.allowRule(forHost: host) {
            decision = "allow"; rule = "allow:\(pattern)"
        } else if policy.mode == "watch" {
            decision = "allow"; rule = "watch"
        } else {
            decision = "deny"; rule = "default-deny"
        }

        record(label: label, pid: pid, host: hostname == host ? nil : hostname, ip: host, port: port,
               proto: proto, decision: decision, rule: rule)

        return decision == "allow" ? .allow() : .drop()
    }

    // MARK: helpers

    /// Returns (label, pid). On macOS the flow carries only an audit token, so we resolve the
    /// executable name through the pid and, when the binary is signed, its signing identifier
    /// (bundle id for apps). Label is "name" or "name (signing.id)" when they differ, so a policy
    /// can match either. Unsigned or ad-hoc Homebrew binaries such as `ollama` match by name.
    private func attribute(_ flow: NEFilterFlow) -> (String, Int?) {
        guard let tokenData = flow.sourceAppAuditToken,
              tokenData.count == MemoryLayout<audit_token_t>.size else { return ("unknown", nil) }
        let token = tokenData.withUnsafeBytes { $0.load(as: audit_token_t.self) }
        let pid = audit_token_to_pid(token)

        var name = "unknown"
        if pid > 0 {
            var buf = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
            if proc_pidpath(pid, &buf, UInt32(buf.count)) > 0 {
                name = (String(cString: buf) as NSString).lastPathComponent
            }
        }
        if let signing = signingIdentifier(for: tokenData), signing.lowercased() != name.lowercased() {
            return ("\(name) (\(signing))", pid > 0 ? Int(pid) : nil)
        }
        return (name, pid > 0 ? Int(pid) : nil)
    }

    private func signingIdentifier(for token: Data) -> String? {
        var code: SecCode?
        let attrs = [kSecGuestAttributeAudit: token] as CFDictionary
        guard SecCodeCopyGuestWithAttributes(nil, attrs, [], &code) == errSecSuccess, let code else { return nil }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else { return nil }
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(staticCode, [], &info) == errSecSuccess,
              let dict = info as? [String: Any] else { return nil }
        return dict[kSecCodeInfoIdentifier as String] as? String
    }

    private func remote(of socket: NEFilterSocketFlow) -> (String, Int) {
        if #available(macOS 15.0, *), let ep = socket.remoteFlowEndpoint, case let .hostPort(h, p) = ep {
            let host: String
            switch h {
            case .ipv4(let a): host = "\(a)"
            case .ipv6(let a): host = "\(a)"
            case .name(let n, _): host = n
            @unknown default: host = "\(h)"
            }
            return (host, Int(p.rawValue))
        }
        if let ep = socket.remoteEndpoint as? NWHostEndpoint {
            return (ep.hostname, Int(ep.port) ?? 0)
        }
        return ("unknown", 0)
    }

    private func record(label: String, pid: Int?, host: String?, ip: String, port: Int,
                        proto: String, decision: String, rule: String) {
        queue.async {
            do {
                _ = try self.receipts.append(source: "ne", process: label, uid: pid, dstHost: host, dstIP: ip,
                                             dstPort: port, proto: proto, direction: "out",
                                             decision: decision, rule: rule, bytesOut: nil)
            } catch {
                os_log("receipt write failed: %{public}@", log: self.log, type: .error, "\(error)")
            }
            os_log("%{public}@ %{public}@ → %{public}@:%d [%{public}@]", log: self.log, type: .info,
                   decision, label, host ?? ip, port, rule)
        }
    }

    private func reloadPolicy() {
        policy = Policy.load(from: SharedPaths.policyDirectory())
        os_log("policy loaded: mode=%{public}@ fenced=%{public}@ allow=%{public}@", log: log, type: .info,
               policy.mode, policy.fencedProcesses.joined(separator: ","), policy.allowHosts.joined(separator: ","))
    }

    /// Reload when the host app rewrites policy.json. Watches the directory because atomic
    /// writes replace the file inode.
    private func watchPolicy() {
        let dir = SharedPaths.policyDirectory()
        let fd = open(dir.path, O_EVTONLY)
        guard fd >= 0 else { return }
        let src = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .rename, .extend], queue: queue)
        src.setEventHandler { [weak self] in self?.reloadPolicy() }
        src.setCancelHandler { close(fd) }
        src.resume()
        policyWatcher = src
    }
}
