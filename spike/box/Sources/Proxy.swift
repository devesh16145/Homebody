// Proxy.swift — a deliberately small HTTP proxy that speaks on a vsock connection from the guest.
// Supports CONNECT host:port (TLS passthrough, no inspection in this spike) and absolute-URI
// plain HTTP (what apt uses). DNS is resolved HERE, on the host; the guest has no resolver.
// Every request becomes a signed receipt via ReceiptLog.

import Foundation

struct BoxPolicy {
    var allow: [String]
    var watch: Bool

    mutating func load(file: String) {
        guard let s = try? String(contentsOfFile: file, encoding: .utf8) else { return }
        for raw in s.split(separator: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty || line.hasPrefix("#") { continue }
            allow.append(line)
        }
    }

    /// Returns the matching pattern or nil. Hostnames are canonicalised first; anything odd is denied.
    func match(host: String) -> String? {
        guard let h = BoxPolicy.canonical(host) else { return nil }
        for p in allow {
            let pat = p.lowercased()
            if pat == h { return p }
            if pat.hasPrefix("*."), h.hasSuffix(String(pat.dropFirst(1))) || h == String(pat.dropFirst(2)) { return p }
        }
        return nil
    }

    /// Lowercase, strip one trailing dot, reject empty, control chars, NUL, spaces, '@', '/', '%' and IP literals
    /// (an allowlist is by name; raw IPs bypass it and are therefore denied).
    static func canonical(_ host: String) -> String? {
        var h = host.lowercased()
        if h.hasSuffix(".") { h.removeLast() }
        if h.isEmpty || h.count > 253 { return nil }
        for u in h.unicodeScalars {
            if u.value < 0x21 || u.value > 0x7e { return nil }
            if "@/%\\#?[]".unicodeScalars.contains(u) { return nil }
        }
        if h.allSatisfy({ $0.isNumber || $0 == "." }) { return nil }   // IPv4 literal
        if h.contains(":") { return nil }                              // IPv6 literal
        return h
    }
}

final class ProxyConnection {
    let fd: Int32
    let policy: BoxPolicy
    let receipts: ReceiptLog
    let label: String
    static let receiptQueue = DispatchQueue(label: "com.sandboxai.box.receipts")

    init(fd: Int32, policy: BoxPolicy, receipts: ReceiptLog, label: String) {
        self.fd = fd; self.policy = policy; self.receipts = receipts; self.label = label
    }

    func run() {
        guard let (head, leftover) = readHead(fd) else { return }
        let lines = head.components(separatedBy: "\r\n")
        guard let request = lines.first else { return }
        let parts = request.split(separator: " ", maxSplits: 2).map(String.init)
        guard parts.count == 3 else { respond(400, "bad request line"); return }
        let method = parts[0], target = parts[1], version = parts[2]

        var host = "", port = 0, forward = Data()
        if method == "CONNECT" {
            let hp = target.split(separator: ":", maxSplits: 1).map(String.init)
            host = hp.first ?? ""; port = hp.count > 1 ? Int(hp[1]) ?? 443 : 443
        } else if let url = URL(string: target), let h = url.host, url.scheme == "http" {
            host = h; port = url.port ?? 80
            // Rewrite absolute-form to origin-form and drop hop-by-hop proxy headers.
            var path = url.path.isEmpty ? "/" : url.path
            if let q = url.query { path += "?" + q }
            var out = "\(method) \(path) \(version)\r\n"
            for l in lines.dropFirst() where !l.isEmpty && !l.lowercased().hasPrefix("proxy-") {
                out += l + "\r\n"
            }
            out += "\r\n"
            forward = out.data(using: .utf8)! + leftover
        } else {
            respond(400, "only CONNECT and absolute http:// requests are supported"); return
        }

        // Decision
        let rule: String
        let decision: String
        if let m = policy.match(host: host) { decision = "allow"; rule = "allow:\(m)" }
        else if policy.watch, BoxPolicy.canonical(host) != nil { decision = "allow"; rule = "watch" }
        else { decision = "deny"; rule = BoxPolicy.canonical(host) == nil ? "deny:malformed-host" : "default-deny" }

        if decision == "deny" {
            record(host: host, ip: "-", port: port, decision: "deny", rule: rule)
            note("DENY  \(method) \(host):\(port)  [\(rule)]")
            respond(403, "Blocked by Sandbox-ai: \(host) is not a trusted destination")
            return
        }

        // Resolve on the host and connect.
        guard let (upstream, ip) = connectUpstream(host: host, port: port) else {
            record(host: host, ip: "-", port: port, decision: "allow", rule: rule + ";connect-failed")
            note("ALLOW \(method) \(host):\(port) but upstream connect failed")
            respond(502, "could not reach \(host)")
            return
        }
        record(host: host, ip: ip, port: port, decision: "allow", rule: rule)
        note("ALLOW \(method) \(host):\(port) → \(ip)  [\(rule)]")

        if method == "CONNECT" {
            writeAll(fd, "HTTP/1.1 200 Connection Established\r\n\r\n".data(using: .utf8)!)
            if !leftover.isEmpty { writeAll(upstream, leftover) }
        } else {
            writeAll(upstream, forward)
        }
        relay(fd, upstream)
        close(upstream)
    }

    // MARK: pieces

    private func readHead(_ fd: Int32) -> (String, Data)? {
        var buf = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while buf.count < 64 * 1024 {
            let n = read(fd, &chunk, chunk.count)
            if n <= 0 { return nil }
            buf.append(contentsOf: chunk[0..<n])
            if let r = buf.range(of: Data("\r\n\r\n".utf8)) {
                let head = String(decoding: buf[buf.startIndex..<r.lowerBound], as: UTF8.self)
                let rest = buf[r.upperBound...]
                return (head, Data(rest))
            }
        }
        return nil
    }

    private func connectUpstream(host: String, port: Int) -> (Int32, String)? {
        var hints = addrinfo(ai_flags: 0, ai_family: AF_UNSPEC, ai_socktype: SOCK_STREAM, ai_protocol: IPPROTO_TCP,
                             ai_addrlen: 0, ai_canonname: nil, ai_addr: nil, ai_next: nil)
        var res: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, String(port), &hints, &res) == 0, let first = res else { return nil }
        defer { freeaddrinfo(res) }
        var p: UnsafeMutablePointer<addrinfo>? = first
        while let ai = p {
            let s = socket(ai.pointee.ai_family, ai.pointee.ai_socktype, ai.pointee.ai_protocol)
            if s >= 0 {
                var tv = timeval(tv_sec: 10, tv_usec: 0)
                setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
                if connect(s, ai.pointee.ai_addr, ai.pointee.ai_addrlen) == 0 {
                    var hostBuf = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                    getnameinfo(ai.pointee.ai_addr, ai.pointee.ai_addrlen, &hostBuf, socklen_t(hostBuf.count), nil, 0, NI_NUMERICHOST)
                    return (s, String(cString: hostBuf))
                }
                close(s)
            }
            p = ai.pointee.ai_next
        }
        return nil
    }

    private func relay(_ a: Int32, _ b: Int32) {
        let group = DispatchGroup()
        for (src, dst) in [(a, b), (b, a)] {
            group.enter()
            Thread {
                var buf = [UInt8](repeating: 0, count: 64 * 1024)
                while true {
                    let n = read(src, &buf, buf.count)
                    if n <= 0 { break }
                    if !self.writeAll(dst, Data(buf[0..<n])) { break }
                }
                shutdown(dst, SHUT_WR)
                group.leave()
            }.start()
        }
        group.wait()
    }

    @discardableResult
    private func writeAll(_ fd: Int32, _ data: Data) -> Bool {
        var off = 0
        return data.withUnsafeBytes { raw -> Bool in
            guard let base = raw.baseAddress else { return true }
            while off < data.count {
                let n = write(fd, base + off, data.count - off)
                if n <= 0 { return false }
                off += n
            }
            return true
        }
    }

    private func respond(_ code: Int, _ text: String) {
        let reason = code == 403 ? "Forbidden" : (code == 502 ? "Bad Gateway" : "Bad Request")
        let body = text + "\n"
        let s = "HTTP/1.1 \(code) \(reason)\r\nContent-Type: text/plain\r\nContent-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n" + body
        writeAll(fd, s.data(using: .utf8)!)
    }

    private func record(host: String, ip: String, port: Int, decision: String, rule: String) {
        ProxyConnection.receiptQueue.sync {
            _ = try? receipts.append(source: "box", process: label, uid: nil, dstHost: host, dstIP: ip, dstPort: port,
                                     proto: "tcp", direction: "out", decision: decision, rule: rule, bytesOut: nil)
        }
    }
}

// MARK: - plain fd relay (used by the SSH-over-vsock bridge)

@discardableResult
func fdWriteAll(_ fd: Int32, _ data: Data) -> Bool {
    var off = 0
    return data.withUnsafeBytes { raw -> Bool in
        guard let base = raw.baseAddress else { return true }
        while off < data.count {
            let n = write(fd, base + off, data.count - off)
            if n <= 0 { return false }
            off += n
        }
        return true
    }
}

/// Copies bytes both ways until either side closes. Blocks the calling thread.
func fdRelay(_ a: Int32, _ b: Int32) {
    let group = DispatchGroup()
    for (src, dst) in [(a, b), (b, a)] {
        group.enter()
        Thread {
            var buf = [UInt8](repeating: 0, count: 64 * 1024)
            while true {
                let n = read(src, &buf, buf.count)
                if n <= 0 { break }
                if !fdWriteAll(dst, Data(buf[0..<n])) { break }
            }
            shutdown(dst, SHUT_WR)
            group.leave()
        }.start()
    }
    group.wait()
}

/// Listens on 127.0.0.1:port and hands every accepted client fd to `onClient` on a new thread.
func tcpListen(port: UInt16, onClient: @escaping (Int32) -> Void) -> Bool {
    let s = socket(AF_INET, SOCK_STREAM, 0)
    var yes: Int32 = 1
    setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
    var addr = sockaddr_in()
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = port.bigEndian
    addr.sin_addr.s_addr = inet_addr("127.0.0.1")
    let bound = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(s, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
    guard bound == 0, listen(s, 16) == 0 else { close(s); return false }
    Thread {
        while true {
            let c = accept(s, nil, nil)
            if c < 0 { continue }
            Thread { onClient(c) }.start()
        }
    }.start()
    return true
}
