// Receipt.swift — signed, hash-chained egress receipts.
// Shared between the fencelog CLI and the Network Extension (compiled into both).
// No dependencies beyond Foundation and CryptoKit.

import Foundation
import CryptoKit

/// Everything that is hashed and signed. Field order does not matter: encoding is canonical (sorted keys).
struct ReceiptBody: Codable, Equatable {
    var seq: UInt64
    var ts: String            // ISO 8601 with fractional seconds, UTC
    var source: String        // "pf" | "ne" | "manual"
    var process: String       // human label: "ollama", "com.ollama.app", "LM Studio"
    var uid: Int?             // POSIX uid of the fenced process when known
    var dstHost: String?      // hostname when known (NE knows it, pf does not)
    var dstIP: String
    var dstPort: Int
    var proto: String         // "tcp" | "udp" | "icmp" | "other"
    var direction: String     // "out" | "in"
    var decision: String      // "allow" | "deny" | "ask-allow" | "ask-deny"
    var rule: String          // rule id or label that produced the decision
    var bytesOut: Int?        // when the enforcement point can count
    var prev: String          // hex sha256 of previous receipt body, or 64 zeros for the first
}

struct Receipt: Codable {
    var body: ReceiptBody
    var hash: String          // hex sha256(canonical(body))
    var sig: String           // base64 Ed25519 signature over the raw hash bytes
}

enum ReceiptError: Error, CustomStringConvertible {
    case badChain(seq: UInt64, expectedPrev: String, gotPrev: String)
    case badHash(seq: UInt64)
    case badSignature(seq: UInt64)
    case badSequence(expected: UInt64, got: UInt64)
    case corruptLine(Int)
    case noKey

    var description: String {
        switch self {
        case .badChain(let s, let e, let g): return "receipt \(s): prev mismatch (expected \(e.prefix(12))…, got \(g.prefix(12))…)"
        case .badHash(let s): return "receipt \(s): body hash does not match"
        case .badSignature(let s): return "receipt \(s): signature invalid"
        case .badSequence(let e, let g): return "sequence gap: expected \(e), got \(g)"
        case .corruptLine(let n): return "line \(n): not a valid receipt"
        case .noKey: return "no signing key; run `fencelog init`"
        }
    }
}

enum Canonical {
    static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return e
    }()
    static let decoder = JSONDecoder()

    static func bytes(_ body: ReceiptBody) throws -> Data { try encoder.encode(body) }

    static func hashHex(_ body: ReceiptBody) throws -> String {
        let d = SHA256.hash(data: try bytes(body))
        return d.map { String(format: "%02x", $0) }.joined()
    }

    static func hexToData(_ hex: String) -> Data? {
        guard hex.count % 2 == 0 else { return nil }
        var out = Data(capacity: hex.count / 2)
        var idx = hex.startIndex
        while idx < hex.endIndex {
            let next = hex.index(idx, offsetBy: 2)
            guard let b = UInt8(hex[idx..<next], radix: 16) else { return nil }
            out.append(b)
            idx = next
        }
        return out
    }

    static let zeroHash = String(repeating: "0", count: 64)

    static func now() -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        f.timeZone = TimeZone(identifier: "UTC")
        return f.string(from: Date())
    }
}

/// Append-only JSONL log with an Ed25519 key beside it.
final class ReceiptLog {
    let directory: URL
    var logURL: URL { directory.appendingPathComponent("receipts.jsonl") }
    var keyURL: URL { directory.appendingPathComponent("signing.key") }
    var pubURL: URL { directory.appendingPathComponent("signing.pub") }

    init(directory: URL) {
        self.directory = directory
    }

    static func defaultDirectory() -> URL {
        if let env = ProcessInfo.processInfo.environment["FENCELOG_DIR"], !env.isEmpty {
            return URL(fileURLWithPath: env, isDirectory: true)
        }
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return base.appendingPathComponent("SandboxAI", isDirectory: true)
    }

    func ensureDirectory() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
    }

    // MARK: keys

    @discardableResult
    func initKey(force: Bool = false) throws -> Curve25519.Signing.PrivateKey {
        try ensureDirectory()
        if !force, let k = try? loadKey() { return k }
        let key = Curve25519.Signing.PrivateKey()
        try key.rawRepresentation.write(to: keyURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: keyURL.path)
        try key.publicKey.rawRepresentation.base64EncodedString().write(to: pubURL, atomically: true, encoding: .utf8)
        return key
    }

    func loadKey() throws -> Curve25519.Signing.PrivateKey {
        guard let raw = try? Data(contentsOf: keyURL) else { throw ReceiptError.noKey }
        return try Curve25519.Signing.PrivateKey(rawRepresentation: raw)
    }

    func loadPublicKey() throws -> Curve25519.Signing.PublicKey {
        if let s = try? String(contentsOf: pubURL, encoding: .utf8),
           let raw = Data(base64Encoded: s.trimmingCharacters(in: .whitespacesAndNewlines)) {
            return try Curve25519.Signing.PublicKey(rawRepresentation: raw)
        }
        return try loadKey().publicKey
    }

    // MARK: read

    func readAll() throws -> [Receipt] {
        guard let data = try? Data(contentsOf: logURL), !data.isEmpty else { return [] }
        var out: [Receipt] = []
        var lineNo = 0
        for line in data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true) {
            lineNo += 1
            guard let r = try? Canonical.decoder.decode(Receipt.self, from: Data(line)) else {
                throw ReceiptError.corruptLine(lineNo)
            }
            out.append(r)
        }
        return out
    }

    func last() throws -> Receipt? {
        // Cheap enough for a spike; a real build keeps a tail index.
        try readAll().last
    }

    // MARK: write

    /// Appends a receipt, filling seq, ts and prev from the chain and signing it.
    @discardableResult
    func append(source: String, process: String, uid: Int?, dstHost: String?, dstIP: String, dstPort: Int,
                proto: String, direction: String, decision: String, rule: String, bytesOut: Int?) throws -> Receipt {
        try ensureDirectory()
        let key = try loadKey()
        let previous = try last()
        var body = ReceiptBody(
            seq: (previous?.body.seq ?? 0) + 1,
            ts: Canonical.now(),
            source: source, process: process, uid: uid,
            dstHost: dstHost, dstIP: dstIP, dstPort: dstPort,
            proto: proto, direction: direction, decision: decision, rule: rule,
            bytesOut: bytesOut,
            prev: previous?.hash ?? Canonical.zeroHash)
        // Normalise optional empties
        if body.dstHost?.isEmpty == true { body.dstHost = nil }
        let hash = try Canonical.hashHex(body)
        let sig = try key.signature(for: Canonical.hexToData(hash)!).base64EncodedString()
        let receipt = Receipt(body: body, hash: hash, sig: sig)
        var line = try Canonical.encoder.encode(receipt)
        line.append(UInt8(ascii: "\n"))
        if FileManager.default.fileExists(atPath: logURL.path) {
            let h = try FileHandle(forWritingTo: logURL)
            defer { try? h.close() }
            try h.seekToEnd()
            try h.write(contentsOf: line)
        } else {
            try line.write(to: logURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: logURL.path)
        }
        return receipt
    }

    // MARK: verify

    /// Verifies hashes, signatures, sequence and chain. Returns the number of receipts checked.
    func verify() throws -> Int {
        let pub = try loadPublicKey()
        let receipts = try readAll()
        var expectedPrev = Canonical.zeroHash
        var expectedSeq: UInt64 = 1
        for r in receipts {
            if r.body.seq != expectedSeq { throw ReceiptError.badSequence(expected: expectedSeq, got: r.body.seq) }
            if r.body.prev != expectedPrev { throw ReceiptError.badChain(seq: r.body.seq, expectedPrev: expectedPrev, gotPrev: r.body.prev) }
            let h = try Canonical.hashHex(r.body)
            if h != r.hash { throw ReceiptError.badHash(seq: r.body.seq) }
            guard let sigData = Data(base64Encoded: r.sig), let hashData = Canonical.hexToData(r.hash),
                  pub.isValidSignature(sigData, for: hashData) else { throw ReceiptError.badSignature(seq: r.body.seq) }
            expectedPrev = r.hash
            expectedSeq += 1
        }
        return receipts.count
    }
}
