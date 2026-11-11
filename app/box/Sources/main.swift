// homebody-box — the Box: a Linux virtual machine with NO network interface.
//
// The machine has exactly one way out: a virtio socket (vsock) to this program, which passes
// every connection, untouched, to the app's gate on this Mac. So everything the Box sends is
// judged by the same rules, asked about with the same questions and written to the same signed
// log as anything else Homebody protects. There is no interface to send a packet on, and no
// name server to ask: both are done by the gate, outside.
//
//   homebody-box run --disk run.raw --state DIR --gate-port 47110
//                    [--seed seed.iso] [--share DIR] [--ssh-port 2222] [--cpus 2] [--mem 2048] [--log FILE]
//
//   --gate-port   the gate (on 127.0.0.1) that the guest's connections are handed to
//   --share       one folder that appears in the guest at /work (and nothing else of this Mac does)
//   --ssh-port    a port on 127.0.0.1 bridged to the guest's SSH over vsock: how commands get in
//
// Ask it to stop with SIGTERM or SIGINT: the guest is told to shut down and given a few seconds.

import Foundation
import Virtualization

let args = Array(CommandLine.arguments.dropFirst())

func die(_ msg: String) -> Never {
    FileHandle.standardError.write(("homebody-box: " + msg + "\n").data(using: .utf8)!)
    exit(1)
}
func note(_ msg: String) {
    FileHandle.standardError.write(("[homebody-box] " + msg + "\n").data(using: .utf8)!)
}
func flag(_ name: String) -> String? {
    guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
    return args[i + 1]
}

guard args.first == "run" else {
    print("homebody-box run --disk FILE --state DIR --gate-port PORT [--seed FILE] [--share DIR] [--ssh-port PORT] [--cpus N] [--mem MiB] [--log FILE]")
    exit(args.isEmpty || args.first == "--help" ? 0 : 2)
}
guard let diskPath = flag("--disk") else { die("--disk is required") }
guard let statePath = flag("--state") else { die("--state is required") }
guard let gatePort = UInt16(flag("--gate-port") ?? "") else { die("--gate-port is required") }
let cpus = Int(flag("--cpus") ?? "2") ?? 2
let memMiB = UInt64(flag("--mem") ?? "2048") ?? 2048
let stateDir = URL(fileURLWithPath: statePath, isDirectory: true)
try? FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true)

// MARK: sockets

func fdWriteAll(_ fd: Int32, _ data: Data) -> Bool {
    var off = 0
    return data.withUnsafeBytes { (p: UnsafeRawBufferPointer) -> Bool in
        while off < data.count {
            let n = write(fd, p.baseAddress! + off, data.count - off)
            if n <= 0 { return false }
            off += n
        }
        return true
    }
}

/// Copy both ways until either side is done.
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

func loopback(_ port: UInt16) -> sockaddr_in {
    var addr = sockaddr_in()
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = port.bigEndian
    addr.sin_addr.s_addr = inet_addr("127.0.0.1")
    return addr
}

/// Connect to a port on this Mac. -1 when nothing answers.
func connectLocal(_ port: UInt16) -> Int32 {
    let s = socket(AF_INET, SOCK_STREAM, 0)
    var addr = loopback(port)
    let ok = withUnsafePointer(to: &addr) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(s, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
    if ok != 0 { close(s); return -1 }
    return s
}

/// Listen on 127.0.0.1:port and hand each client to `onClient` on its own thread.
func listenLocal(_ port: UInt16, onClient: @escaping (Int32) -> Void) -> Bool {
    let s = socket(AF_INET, SOCK_STREAM, 0)
    var yes: Int32 = 1
    setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
    var addr = loopback(port)
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

// MARK: the machine

let config = VZVirtualMachineConfiguration()
config.cpuCount = max(VZVirtualMachineConfiguration.minimumAllowedCPUCount, min(cpus, VZVirtualMachineConfiguration.maximumAllowedCPUCount))
config.memorySize = max(VZVirtualMachineConfiguration.minimumAllowedMemorySize, min(memMiB * 1024 * 1024, VZVirtualMachineConfiguration.maximumAllowedMemorySize))

// The same machine identity across starts, so the guest sees one computer.
let platform = VZGenericPlatformConfiguration()
let idURL = stateDir.appendingPathComponent("machine-id")
if let d = try? Data(contentsOf: idURL), let id = VZGenericMachineIdentifier(dataRepresentation: d) {
    platform.machineIdentifier = id
} else {
    let id = VZGenericMachineIdentifier()
    try? id.dataRepresentation.write(to: idURL)
    platform.machineIdentifier = id
}
config.platform = platform

let efi = VZEFIBootLoader()
let varsURL = stateDir.appendingPathComponent("efivars")
if FileManager.default.fileExists(atPath: varsURL.path) {
    efi.variableStore = VZEFIVariableStore(url: varsURL)
} else {
    do { efi.variableStore = try VZEFIVariableStore(creatingVariableStoreAt: varsURL) } catch { die("efi variables: \(error)") }
}
config.bootLoader = efi

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

// No network device. This is the point of the Box.
config.networkDevices = []
// The one channel out.
config.socketDevices = [VZVirtioSocketDeviceConfiguration()]

// One folder of this Mac, and only that one, at /work in the guest.
if let share = flag("--share") {
    let fs = VZVirtioFileSystemDeviceConfiguration(tag: "work")
    fs.share = VZSingleDirectoryShare(directory: VZSharedDirectory(url: URL(fileURLWithPath: share), readOnly: false))
    config.directorySharingDevices = [fs]
}

// The guest's console goes to a log file (or nowhere); nothing is typed into it.
let hostToGuest = Pipe()
let guestToHost = Pipe()
let serial = VZVirtioConsoleDeviceSerialPortConfiguration()
serial.attachment = VZFileHandleSerialPortAttachment(fileHandleForReading: hostToGuest.fileHandleForReading, fileHandleForWriting: guestToHost.fileHandleForWriting)
config.serialPorts = [serial]
config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
// Lets the guest hand memory it is not using back to this Mac.
config.memoryBalloonDevices = [VZVirtioTraditionalMemoryBalloonDeviceConfiguration()]

let consoleLog: FileHandle? = {
    guard let p = flag("--log") else { return nil }
    FileManager.default.createFile(atPath: p, contents: nil)
    return FileHandle(forWritingAtPath: p)
}()
guestToHost.fileHandleForReading.readabilityHandler = { h in
    let d = h.availableData
    if !d.isEmpty { consoleLog?.write(d) }
}

do { try config.validate() } catch { die("this configuration cannot run: \(error)") }

let vmQueue = DispatchQueue(label: "app.homebody.box.vm")
let vm = VZVirtualMachine(configuration: config, queue: vmQueue)

final class VMDelegate: NSObject, VZVirtualMachineDelegate {
    func guestDidStop(_ virtualMachine: VZVirtualMachine) { note("stopped"); exit(0) }
    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) { note("stopped with an error: \(error.localizedDescription)"); exit(1) }
}
let vmDelegate = VMDelegate()
vm.delegate = vmDelegate

// Guest connections on vsock 3128 are handed to the gate, byte for byte.
final class GateListener: NSObject, VZVirtioSocketListenerDelegate {
    let gatePort: UInt16
    init(gatePort: UInt16) { self.gatePort = gatePort }
    func listener(_ listener: VZVirtioSocketListener, shouldAcceptNewConnection connection: VZVirtioSocketConnection, from socketDevice: VZVirtioSocketDevice) -> Bool {
        let guest = connection.fileDescriptor
        Thread {
            let gate = connectLocal(self.gatePort)
            if gate < 0 {
                _ = fdWriteAll(guest, "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n".data(using: .utf8)!)
            } else {
                fdRelay(guest, gate)
                close(gate)
            }
            connection.close()
        }.start()
        return true
    }
}
let gateListener = GateListener(gatePort: gatePort)
let vzListener = VZVirtioSocketListener()
vzListener.delegate = gateListener

// Stop when asked: tell the guest to shut down, and stop it outright if it has not after a while.
func stopSoon() {
    vmQueue.async {
        if vm.canRequestStop { try? vm.requestStop() }
        vmQueue.asyncAfter(deadline: .now() + 8) {
            if vm.canStop { vm.stop { _ in exit(0) } } else { exit(0) }
        }
    }
}
signal(SIGTERM, SIG_IGN)
signal(SIGINT, SIG_IGN)
var sources: [DispatchSourceSignal] = []
for sig in [SIGTERM, SIGINT] {
    let src = DispatchSource.makeSignalSource(signal: sig, queue: .global())
    src.setEventHandler { note("asked to stop"); stopSoon() }
    src.resume()
    sources.append(src)
}

vmQueue.async {
    vm.start { result in
        switch result {
        case .failure(let e): die("could not start: \(e.localizedDescription)")
        case .success:
            guard let dev = vm.socketDevices.first as? VZVirtioSocketDevice else { die("no socket device") }
            dev.setSocketListener(vzListener, forPort: 3128)
            note("running: \(config.cpuCount) cpu, \(memMiB) MiB, no network interface, way out: gate on port \(gatePort)")
            if let p = UInt16(flag("--ssh-port") ?? "0"), p > 0 {
                let ok = listenLocal(p) { client in
                    vmQueue.async {
                        dev.connect(toPort: 22) { result in
                            switch result {
                            case .success(let conn):
                                Thread { fdRelay(client, conn.fileDescriptor); conn.close(); close(client) }.start()
                            case .failure:
                                close(client)
                            }
                        }
                    }
                }
                note(ok ? "commands: 127.0.0.1:\(p)" : "could not listen on 127.0.0.1:\(p)")
                if !ok { exit(3) }
            }
        }
    }
}
dispatchMain()
