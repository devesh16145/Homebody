// fencelog — CLI for signed, hash-chained egress receipts.
//
//   fencelog init                      create the Ed25519 signing key
//   fencelog append [flags]            append one receipt (see --help)
//   fencelog ingest-pflog [flags]      read `tcpdump -i pflog0` lines on stdin, append a receipt per line
//   fencelog tail [-n N]               print the last N receipts in plain words
//   fencelog verify                    verify hashes, signatures and chain
//   fencelog proof --process NAME [--since ISO8601]   print a zero-egress statement
//   fencelog export --out FILE         write a verifiable bundle (receipts + public key)
//
// Log location: $FENCELOG_DIR or ~/Library/Application Support/SandboxAI/

import Foundation
import CryptoKit

let args = Array(CommandLine.arguments.dropFirst())
let log = ReceiptLog(directory: ReceiptLog.defaultDirectory())

func die(_ msg: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(code)
}

func flag(_ name: String, in a: [String]) -> String? {
    guard let i = a.firstIndex(of: name), i + 1 < a.count else { return nil }
    return a[i + 1]
}

func usage() -> Never {
    print("""
    fencelog — signed, hash-chained egress receipts

      init
      append --process P --dst IP --port N [--host H] [--uid U] [--proto tcp|udp] [--dir out|in]
             [--decision allow|deny|ask-allow|ask-deny] [--rule R] [--source pf|ne|manual] [--bytes B]
      ingest-pflog [--process P] [--uid U]        (stdin: `tcpdump -n -e -ttt -l -i pflog0`)
      tail [-n N]
      verify
      proof --process P [--since ISO8601]
      export --out FILE

    Log dir: \(log.directory.path)
    """)
    exit(0)
}

func humanLine(_ r: Receipt) -> String {
    let b = r.body
    let verb: String
    switch b.decision {
    case "allow", "ask-allow": verb = "Allowed"
    case "deny", "ask-deny": verb = "Blocked"
    default: verb = b.decision
    }
    let dest = b.dstHost.map { "\($0) (\(b.dstIP))" } ?? b.dstIP
    let bytes = b.bytesOut.map { ", \($0) bytes" } ?? ""
    return "\(b.ts)  \(verb)  \(b.process) → \(dest):\(b.dstPort)/\(b.proto)  [\(b.rule)]\(bytes)"
}

guard let cmd = args.first else { usage() }
let rest = Array(args.dropFirst())
if rest.contains("--help") || rest.contains("-h") { usage() }

switch cmd {
case "init":
    do {
        let key = try log.initKey(force: rest.contains("--force"))
        print("key ready: \(log.keyURL.path)")
        print("public:    \(key.publicKey.rawRepresentation.base64EncodedString())")
    } catch { die("init failed: \(error)") }

case "append":
    guard let process = flag("--process", in: rest), let dst = flag("--dst", in: rest),
          let portS = flag("--port", in: rest), let port = Int(portS) else {
        die("append needs --process, --dst and --port")
    }
    do {
        let r = try log.append(
            source: flag("--source", in: rest) ?? "manual",
            process: process,
            uid: flag("--uid", in: rest).flatMap(Int.init),
            dstHost: flag("--host", in: rest),
            dstIP: dst, dstPort: port,
            proto: flag("--proto", in: rest) ?? "tcp",
            direction: flag("--dir", in: rest) ?? "out",
            decision: flag("--decision", in: rest) ?? "deny",
            rule: flag("--rule", in: rest) ?? "default-deny",
            bytesOut: flag("--bytes", in: rest).flatMap(Int.init))
        print(humanLine(r))
    } catch { die("append failed: \(error)") }

case "ingest-pflog":
    // Parses lines like:
    //  00:00:01.234567 rule 0.com.sandboxai/fence.0/0(match): block out on en0: 10.0.0.5.52345 > 93.184.216.34.443: Flags [S], seq ...
    //  00:00:00.000100 rule 2/0(match): pass out on en0: 10.0.0.5.60000 > 1.1.1.1.53: UDP, length 40
    let processLabel = flag("--process", in: rest) ?? "fenced"
    let uid = flag("--uid", in: rest).flatMap(Int.init)
    // pflog carries every logged rule on the box, including Apple's own anchors (multicast, mDNS…).
    // Only lines from our anchor are the fenced process's traffic.
    let anchorTag = flag("--anchor", in: rest) ?? "fence"
    let re = try! NSRegularExpression(pattern: #"rule ([^:]+?)(?:\(match\))?: (block|pass) (in|out) on ([^:]+): (.+?)\.(\d+) > (.+?)\.(\d+): (.*)$"#)
    var count = 0
    while let line = readLine(strippingNewline: true) {
        let ns = line as NSString
        guard let m = re.firstMatch(in: line, range: NSRange(location: 0, length: ns.length)) else { continue }
        let rule = ns.substring(with: m.range(at: 1))
        guard rule.contains(anchorTag) else { continue }
        let action = ns.substring(with: m.range(at: 2))
        let dir = ns.substring(with: m.range(at: 3))
        let dstIP = ns.substring(with: m.range(at: 7))
        let dstPort = Int(ns.substring(with: m.range(at: 8))) ?? 0
        let tail = ns.substring(with: m.range(at: 9))
        let proto = tail.hasPrefix("UDP") ? "udp" : (tail.contains("Flags") ? "tcp" : (tail.contains("ICMP") ? "icmp" : "other"))
        // Only the SYN of a TCP flow counts as a new connection; skip retransmits and later segments.
        if proto == "tcp" && !tail.contains("Flags [S]") { continue }
        do {
            let r = try log.append(source: "pf", process: processLabel, uid: uid, dstHost: nil,
                                   dstIP: dstIP, dstPort: dstPort, proto: proto, direction: dir,
                                   decision: action == "block" ? "deny" : "allow",
                                   rule: "pf:" + rule, bytesOut: nil)
            count += 1
            print(humanLine(r))
            fflush(stdout)
        } catch { die("append failed: \(error)") }
    }
    FileHandle.standardError.write("ingested \(count) receipts\n".data(using: .utf8)!)

case "tail":
    let n = flag("-n", in: rest).flatMap(Int.init) ?? 20
    do {
        let all = try log.readAll()
        for r in all.suffix(n) { print(humanLine(r)) }
        if all.isEmpty { print("(no receipts yet)") }
    } catch { die("read failed: \(error)") }

case "verify":
    do {
        let n = try log.verify()
        print("OK: \(n) receipts, chain intact, all signatures valid")
    } catch { die("VERIFY FAILED: \(error)", code: 2) }

case "proof":
    guard let process = flag("--process", in: rest) else { die("proof needs --process") }
    do {
        _ = try log.verify()
        let all = try log.readAll()
        let since = flag("--since", in: rest)
        let fmt = ISO8601DateFormatter(); fmt.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let sinceDate = since.flatMap { fmt.date(from: $0) ?? ISO8601DateFormatter().date(from: $0) }
        let mine = all.filter { r in
            r.body.process.localizedCaseInsensitiveContains(process) &&
            (sinceDate == nil || (fmt.date(from: r.body.ts) ?? .distantPast) >= sinceDate!)
        }
        let allowed = mine.filter { $0.body.decision.hasSuffix("allow") && $0.body.direction == "out" }
        let blocked = mine.filter { $0.body.decision.hasSuffix("deny") }
        let allowedNonLocal = allowed.filter { !["127.0.0.1", "::1", "localhost"].contains($0.body.dstIP) }
        let first = mine.first?.body.ts ?? "(none)"
        let pub = try log.loadPublicKey().rawRepresentation.base64EncodedString()
        print("Proof for \(process)")
        print("  period:            \(since ?? first) → \(Canonical.now())")
        print("  receipts:          \(mine.count) (chain verified)")
        print("  blocked attempts:  \(blocked.count)")
        print("  allowed external:  \(allowedNonLocal.count)")
        if allowedNonLocal.isEmpty {
            print("  statement:         Nothing left this machine from \(process) in this period.")
        } else {
            print("  statement:         \(allowedNonLocal.count) external connections were allowed by rule:")
            for r in allowedNonLocal.prefix(10) { print("                     " + humanLine(r)) }
        }
        print("  signing key:       \(pub)")
        print("  not covered:       traffic from other processes, covert channels, host compromise. See docs.")
    } catch { die("proof failed: \(error)", code: 2) }

case "export":
    guard let out = flag("--out", in: rest) else { die("export needs --out FILE") }
    do {
        _ = try log.verify()
        let receipts = try Data(contentsOf: log.logURL)
        let pub = try log.loadPublicKey().rawRepresentation.base64EncodedString()
        let bundle: [String: Any] = [
            "format": "sandboxai-receipts-v1",
            "exportedAt": Canonical.now(),
            "publicKey": pub,
            "receiptsJSONL": String(decoding: receipts, as: UTF8.self)
        ]
        let data = try JSONSerialization.data(withJSONObject: bundle, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: URL(fileURLWithPath: out))
        print("exported \(receipts.split(separator: UInt8(ascii: "\n")).count) receipts to \(out)")
    } catch { die("export failed: \(error)") }

default:
    usage()
}
