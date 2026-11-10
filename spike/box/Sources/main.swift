// sai-box — Box spike: boot a Linux VM with NO network interface on Apple silicon via
// Virtualization.framework, expose a vsock port that a host-side allowlist proxy answers,
// share one directory into the guest, and attach the guest console to this terminal.
//
//   sai-box run --disk disk.raw [--seed seed.iso] [--share DIR] [--cpus 2] [--mem 2048]
//               [--allow HOST ...] [--policy FILE] [--watch] [--state DIR]
//
// The guest reaches the internet only by talking HTTP-proxy to vsock CID 2 port 3128.
// Every request is allowed or denied by hostname on the host and written as a signed receipt
// (same format as the Fence spike) to $FENCELOG_DIR or ~/Library/Application Support/SandboxAI.

import Foundation
import AppKit
import Virtualization

let args = Array(CommandLine.arguments.dropFirst())

func die(_ msg: String) -> Never {
    FileHandle.standardError.write(("sai-box: " + msg + "\n").data(using: .utf8)!)
    exit(1)
}
func note(_ msg: String) {
    FileHandle.standardError.write(("[sai-box] " + msg + "\n").data(using: .utf8)!)
}
func flag(_ name: String) -> String? {
    guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
    return args[i + 1]
}
func flags(_ name: String) -> [String] {
    var out: [String] = []
    var i = 0
    while i < args.count {
        if args[i] == name, i + 1 < args.count { out.append(args[i + 1]); i += 1 }
        i += 1
    }
    return out
}

// `proxy` mode: run only the allowlist proxy on a TCP port, no VM. For testing the policy
// and receipts with curl before a guest exists:  curl -x http://127.0.0.1:3128 https://example.com
if args.first == "proxy" {
    let port = UInt16(flag("--port") ?? "3128") ?? 3128
    let proxyLabel = flag("--label") ?? "proxy"
    var pol = BoxPolicy(allow: flags("--allow"), watch: args.contains("--watch"))
    if let p = flag("--policy") { pol.load(file: p) }
    let log = ReceiptLog(directory: ReceiptLog.defaultDirectory())
    do { try log.initKey() } catch { die("receipt key: \(error)") }
    let s = socket(AF_INET, SOCK_STREAM, 0)
    var yes: Int32 = 1
    setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
    var addr = sockaddr_in()
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = port.bigEndian
    addr.sin_addr.s_addr = inet_addr("127.0.0.1")
    let bound = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(s, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
    guard bound == 0, listen(s, 64) == 0 else { die("cannot listen on 127.0.0.1:\(port)") }
    note("proxy listening on 127.0.0.1:\(port)  policy: \(pol.watch ? "WATCH" : "STRICT") allow=\(pol.allow.joined(separator: ","))")
    while true {
        let c = accept(s, nil, nil)
        if c < 0 { continue }
        Thread { ProxyConnection(fd: c, policy: pol, receipts: log, label: proxyLabel).run(); close(c) }.start()
    }
}

guard args.first == "run" else {
    print("""
    sai-box run   --disk disk.raw [--seed seed.iso] [--share DIR] [--cpus N] [--mem MiB]
                  [--allow HOST]... [--policy FILE] [--watch] [--state DIR] [--name NAME] [--gui] [--log FILE] [--no-balloon] [--ssh-port N]
    sai-box proxy [--port 3128] [--label NAME] [--allow HOST]... [--policy FILE] [--watch]   (no VM; test with curl -x)
    """)
    exit(0)
}

guard let diskPath = flag("--disk") else { die("--disk is required") }
let cpus = Int(flag("--cpus") ?? "2") ?? 2
let memMiB = UInt64(flag("--mem") ?? "2048") ?? 2048
let stateDir = URL(fileURLWithPath: flag("--state") ?? (NSHomeDirectory() + "/Library/Application Support/SandboxAI/box"), isDirectory: true)
let vmName = flag("--name") ?? "box"
try? FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true)

// MARK: policy + receipts

var policy = BoxPolicy(allow: flags("--allow"), watch: args.contains("--watch"))
if let p = flag("--policy") { policy.load(file: p) }
let receipts = ReceiptLog(directory: ReceiptLog.defaultDirectory())
do { try receipts.initKey() } catch { die("receipt key: \(error)") }
note("policy: \(policy.watch ? "WATCH (allow all, log all)" : "STRICT") allow=\(policy.allow.joined(separator: ","))")

// MARK: configuration

let config = VZVirtualMachineConfiguration()
config.cpuCount = max(VZVirtualMachineConfiguration.minimumAllowedCPUCount, min(cpus, VZVirtualMachineConfiguration.maximumAllowedCPUCount))
config.memorySize = max(VZVirtualMachineConfiguration.minimumAllowedMemorySize, min(memMiB * 1024 * 1024, VZVirtualMachineConfiguration.maximumAllowedMemorySize))

// Platform identity persisted so the guest sees the same machine across boots.
let platform = VZGenericPlatformConfiguration()
let idURL = stateDir.appendingPathComponent("\(vmName).machine-id")
if let d = try? Data(contentsOf: idURL), let id = VZGenericMachineIdentifier(dataRepresentation: d) {
    platform.machineIdentifier = id
} else {
    let id = VZGenericMachineIdentifier()
    try? id.dataRepresentation.write(to: idURL)
    platform.machineIdentifier = id
}
config.platform = platform

// EFI boot (cloud images are EFI-bootable; direct kernel boot would need an uncompressed Image).
let efi = VZEFIBootLoader()
let varsURL = stateDir.appendingPathComponent("\(vmName).efivars")
if FileManager.default.fileExists(atPath: varsURL.path) {
    efi.variableStore = VZEFIVariableStore(url: varsURL)
} else {
    do { efi.variableStore = try VZEFIVariableStore(creatingVariableStoreAt: varsURL) } catch { die("efi vars: \(error)") }
}
config.bootLoader = efi

// Storage: root disk (rw) + optional cloud-init seed (ro).
var storage: [VZStorageDeviceConfiguration] = []
do {
    let root = try VZDiskImageStorageDeviceAttachment(url: URL(fileURLWithPath: diskPath), readOnly: false)
    storage.append(VZVirtioBlockDeviceConfiguration(attachment: root))
    if let seed = flag("--seed") {
        let s = try VZDiskImageStorageDeviceAttachment(url: URL(fileURLWithPath: seed), readOnly: true)
        storage.append(VZVirtioBlockDeviceConfiguration(attachment: s))
    }
} catch { die("disk: \(error)") }
config.storageDevices = storage

// NO network device. This is the point of the Box.
config.networkDevices = []

// vsock: the only channel out.
config.socketDevices = [VZVirtioSocketDeviceConfiguration()]

// Shared directory (virtiofs tag "sai").
if let share = flag("--share") {
    let fs = VZVirtioFileSystemDeviceConfiguration(tag: "sai")
    fs.share = VZSingleDirectoryShare(directory: VZSharedDirectory(url: URL(fileURLWithPath: share), readOnly: false))
    config.directorySharingDevices = [fs]
}

// Console: serial port bridged to pipes so we can scan for boot markers and still echo to the terminal.
let hostToGuest = Pipe()
let guestToHost = Pipe()
let serial = VZVirtioConsoleDeviceSerialPortConfiguration()
serial.attachment = VZFileHandleSerialPortAttachment(fileHandleForReading: hostToGuest.fileHandleForReading,
                                                     fileHandleForWriting: guestToHost.fileHandleForWriting)
config.serialPorts = [serial]

config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
if !args.contains("--no-balloon") { config.memoryBalloonDevices = [VZVirtioTraditionalMemoryBalloonDeviceConfiguration()] }

// --gui: a window showing the guest's virtual display (tty0), for debugging boot and as a product preview.
let gui = args.contains("--gui")
if gui {
    let gfx = VZVirtioGraphicsDeviceConfiguration()
    gfx.scanouts = [VZVirtioGraphicsScanoutConfiguration(widthInPixels: 1280, heightInPixels: 800)]
    config.graphicsDevices = [gfx]
    config.keyboards = [VZUSBKeyboardConfiguration()]
    config.pointingDevices = [VZUSBScreenCoordinatePointingDeviceConfiguration()]
}

// --log FILE: tee the guest console to a file.
let consoleLog: FileHandle? = {
    guard let p = flag("--log") else { return nil }
    FileManager.default.createFile(atPath: p, contents: nil)
    return FileHandle(forWritingAtPath: p)
}()

do { try config.validate() } catch { die("invalid configuration: \(error)") }

// MARK: run

// VZVirtualMachineView requires the VM to live on the main queue.
let vmQueue = gui ? DispatchQueue.main : DispatchQueue(label: "com.sandboxai.box.vm")
let vm = VZVirtualMachine(configuration: config, queue: vmQueue)

final class VMDelegate: NSObject, VZVirtualMachineDelegate {
    func guestDidStop(_ virtualMachine: VZVirtualMachine) { note("guest stopped"); restoreTerminal(); exit(0) }
    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) { note("guest error: \(error)"); restoreTerminal(); exit(1) }
}
let vmDelegate = VMDelegate()
vm.delegate = vmDelegate

// Terminal raw mode so the guest console gets keystrokes unmodified.
var savedTermios = termios()
var rawModeOn = false
func enterRawMode() {
    guard isatty(STDIN_FILENO) != 0 else { return }
    tcgetattr(STDIN_FILENO, &savedTermios)
    var raw = savedTermios
    cfmakeraw(&raw)
    tcsetattr(STDIN_FILENO, TCSANOW, &raw)
    rawModeOn = true
}
func restoreTerminal() { if rawModeOn { tcsetattr(STDIN_FILENO, TCSANOW, &savedTermios) } }

let startedAt = Date()
var sawLogin = false

// guest → terminal, scanning for the login prompt to report boot time.
guestToHost.fileHandleForReading.readabilityHandler = { h in
    let d = h.availableData
    if d.isEmpty { return }
    FileHandle.standardOutput.write(d)
    consoleLog?.write(d)
    if !sawLogin, let s = String(data: d, encoding: .utf8), s.contains("login:") || s.contains("sai-ready") {
        sawLogin = true
        let ms = Int(Date().timeIntervalSince(startedAt) * 1000)
        note("guest reached login prompt \(ms) ms after start")
    }
}
// terminal → guest. Ctrl-] (0x1d) detaches and stops the VM.
let stdinReader = Thread {
    var buf = [UInt8](repeating: 0, count: 1024)
    while true {
        let n = read(STDIN_FILENO, &buf, buf.count)
        if n <= 0 { break }
        if buf[0..<n].contains(0x1d) {
            note("Ctrl-] pressed, stopping VM")
            vmQueue.async { vm.stop { _ in restoreTerminal(); exit(0) } }
            break
        }
        hostToGuest.fileHandleForWriting.write(Data(buf[0..<n]))
    }
}

// vsock listener → proxy.
final class SocketListener: NSObject, VZVirtioSocketListenerDelegate {
    var live: [VZVirtioSocketConnection] = []
    let lock = NSLock()
    func listener(_ listener: VZVirtioSocketListener, shouldAcceptNewConnection connection: VZVirtioSocketConnection,
                  from socketDevice: VZVirtioSocketDevice) -> Bool {
        lock.lock(); live.append(connection); lock.unlock()
        let fd = connection.fileDescriptor
        Thread {
            ProxyConnection(fd: fd, policy: policy, receipts: receipts, label: "box:\(vmName)").run()
            connection.close()
            self.lock.lock(); self.live.removeAll { $0 === connection }; self.lock.unlock()
        }.start()
        return true
    }
}
let socketListener = SocketListener()
let vzListener = VZVirtioSocketListener()
vzListener.delegate = socketListener

var guiWindow: NSWindow?
signal(SIGINT) { _ in restoreTerminal(); exit(130) }

vmQueue.async {
    vm.start { result in
        switch result {
        case .failure(let e): die("start failed: \(e)")
        case .success:
            note("VM started: \(cpus) cpu, \(memMiB) MiB, no NIC, vsock:3128 → proxy, \(flag("--share") != nil ? "share tag=sai" : "no share")")
            note("Ctrl-] stops the VM")
            if let dev = vm.socketDevices.first as? VZVirtioSocketDevice {
                dev.setSocketListener(vzListener, forPort: 3128)
                // --ssh-port N: tunnel 127.0.0.1:N on the host to vsock port 22 in the guest, where a small
                // forwarder hands the stream to sshd. No NIC is involved; this is the host→guest command channel.
                if let p = UInt16(flag("--ssh-port") ?? "0"), p > 0 {
                    let ok = tcpListen(port: p) { client in
                        vmQueue.async {
                            dev.connect(toPort: 22) { result in
                                switch result {
                                case .success(let conn):
                                    Thread { fdRelay(client, conn.fileDescriptor); conn.close(); close(client) }.start()
                                case .failure(let e):
                                    note("ssh bridge: guest refused vsock:22 (\(e.localizedDescription))"); close(client)
                                }
                            }
                        }
                    }
                    note(ok ? "ssh bridge: 127.0.0.1:\(p) → guest vsock:22  (ssh -p \(p) root@127.0.0.1)" : "ssh bridge: could not listen on 127.0.0.1:\(p)")
                }
            } else { note("no vsock device?!") }
            enterRawMode()
            stdinReader.start()
            if gui {
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 800),
                                      styleMask: [.titled, .closable, .resizable, .miniaturizable],
                                      backing: .buffered, defer: false)
                window.title = "Sandbox-ai Box — \(vmName)"
                let view = VZVirtualMachineView()
                view.virtualMachine = vm
                view.capturesSystemKeys = false
                window.contentView = view
                window.center()
                window.makeKeyAndOrderFront(nil)
                NSApplication.shared.activate(ignoringOtherApps: true)
                guiWindow = window
            }
        }
    }
}

if gui {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    app.run()
} else {
    dispatchMain()
}
