import SwiftUI
import Darwin

/// Bergamota on the phone: the Mac app's Performance mode (same PerfCatalog), plus Respring and Reboot. It runs sandboxed
/// and asks the root helper bergamotad (tools/sweetnet/bergamotad.c) to read and write the switches.
@main
struct BergamotaPhoneApp: App {
    var body: some Scene { WindowGroup { PerformanceScreen() } }
}

/// PerfCatalog.commands quotes its arguments with this (the Mac app has it in Core/Shell.swift).
func shellQuote(_ s: String) -> String {
    if !s.isEmpty && s.allSatisfy({ $0.isLetter || $0.isNumber || "-_./=:@%+,".contains($0) }) { return s }
    return "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
}

/// One request to bergamotad (tools/sweetnet/bergamotad.c), the root helper on 127.0.0.1:7307: this app launches only
/// sandboxed, so the helper writes the switches, resprings and reboots for it. Returns the reply lines, nil if it is not running.
func helper(_ request: String) -> [String]? {
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else { return nil }
    defer { close(fd) }
    var tv = timeval(tv_sec: 3, tv_usec: 0)
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
    var a = sockaddr_in()
    a.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    a.sin_family = sa_family_t(AF_INET)
    a.sin_port = in_port_t(7307).bigEndian
    a.sin_addr.s_addr = inet_addr("127.0.0.1")
    let connected = withUnsafePointer(to: &a) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
    guard connected == 0 else { return nil }
    let line = request + "\n"
    _ = line.withCString { write(fd, $0, strlen($0)) }
    var data = [UInt8]()
    var buf = [UInt8](repeating: 0, count: 4096)
    while true {
        let n = read(fd, &buf, buf.count)
        if n <= 0 { break }
        data += buf[0..<n]
    }
    return String(decoding: data, as: UTF8.self).split(separator: "\n").map(String.init)
}

@Observable
@MainActor
final class PhoneSwitches {
    var phone: [String: Int] = [:]
    var wanted: [String: Int] = [:]
    var error: String?
    var pendingRespring = false
    var pendingReboot = false

    var changed: [PerfOption] { PerfCatalog.options.filter { wanted[$0.id] != nil && wanted[$0.id] != phone[$0.id] } }
    var activePreset: PerfPreset? { PerfCatalog.presets.first { p in p.choices.allSatisfy { wanted[$0.key] == $0.value } } }
    private let notRunning = "The Bergamota helper (bergamotad) is not running on the phone."

    func read() {
        guard let lines = helper("get") else { error = notRunning; return }
        error = nil
        var present: [String: String] = [:]
        for l in lines where l.contains("=") {
            let parts = l.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            let hex = Array(parts[1])
            var bytes = [UInt8]()
            var i = 0
            while i + 1 < hex.count { bytes.append(UInt8(String(hex[i...i + 1]), radix: 16) ?? 0); i += 2 }
            present[String(parts[0])] = String(decoding: bytes, as: UTF8.self)
        }
        var found: [String: Int] = [:]
        for o in PerfCatalog.options { found[o.id] = PerfCatalog.choiceIndex(o, contents: present[o.id]) }
        phone = found
        for (k, v) in found where wanted[k] == nil { wanted[k] = v }
    }

    func apply() {
        error = nil
        for o in changed {
            guard let i = wanted[o.id], o.choices.indices.contains(i) else { continue }
            let request: String
            if let content = o.choices[i].content {
                request = "set \(o.id) " + content.utf8.map { String(format: "%02x", $0) }.joined()
            } else {
                request = "rm \(o.id)"
            }
            guard let r = helper(request) else { error = notRunning; return }
            if r.first != "ok" { error = "\(o.title): \(r.first ?? "no answer")"; continue }
            if o.applies == .respring { pendingRespring = true } else { pendingReboot = true }
        }
        read()
    }

    func respring() {
        guard let r = helper("respring") else { error = notRunning; return }
        if r.first == "ok" { pendingRespring = false } else { error = r.first }
    }

    /// A normal reboot (sweetreboot normal): the phone comes up on its flashed boot image. For a worn power button.
    func reboot() {
        guard let r = helper("reboot") else { error = notRunning; return }
        if r.first != "ok" { error = r.first }
    }
}

struct PerformanceScreen: View {
    @State private var s = PhoneSwitches()
    @State private var confirmReboot = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Mode") {
                    ForEach(PerfCatalog.presets) { p in
                        Button { for (k, v) in p.choices { s.wanted[k] = v } } label: {
                            HStack(alignment: .top) {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(p.title).font(.headline).foregroundStyle(.primary)
                                    Text(p.detail).font(.footnote).foregroundStyle(.secondary)
                                }
                                Spacer()
                                if s.activePreset?.id == p.id { Image(systemName: "checkmark").foregroundStyle(.tint) }
                            }
                        }
                        .accessibilityAddTraits(s.activePreset?.id == p.id ? .isSelected : [])
                    }
                }
                ForEach(groups, id: \.self) { g in
                    Section(g) {
                        ForEach(PerfCatalog.options.filter { $0.group == g }) { o in optionRow(o) }
                    }
                }
                Section {
                    // two taps instead of a confirmation sheet: sheets and alerts do not show in this app on this phone
                    Button(role: .destructive) {
                        if confirmReboot { s.reboot() } else { confirmReboot = true }
                    } label: {
                        Label(confirmReboot ? "Tap again to reboot" : "Reboot phone", systemImage: "arrow.triangle.2.circlepath")
                    }
                } footer: {
                    Text(s.pendingReboot ? "backboardd reads its switches when it starts: reboot for them to take effect." : "A normal reboot, onto the boot image flashed to the phone.")
                }
                if let e = s.error { Section { Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) } }
            }
            .navigationTitle("Performance")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    if s.pendingRespring { Button("Respring") { s.respring() } }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(s.changed.isEmpty ? "Apply" : "Apply (\(s.changed.count))") { s.apply() }.disabled(s.changed.isEmpty)
                }
            }
            .refreshable { s.read() }
            .onAppear { s.read() }
        }
    }

    private var groups: [String] {
        var seen: [String] = []
        for o in PerfCatalog.options where !seen.contains(o.group) { seen.append(o.group) }
        return seen
    }

    @ViewBuilder
    private func optionRow(_ o: PerfOption) -> some View {
        let binding = Binding<Int>(get: { s.wanted[o.id] ?? 0 }, set: { s.wanted[o.id] = $0 })
        VStack(alignment: .leading, spacing: 4) {
            if o.isFlag {
                Toggle(o.title, isOn: Binding(get: { binding.wrappedValue == 1 }, set: { binding.wrappedValue = $0 ? 1 : 0 }))
            } else {
                Picker(o.title, selection: binding) {
                    ForEach(Array(o.choices.enumerated()), id: \.offset) { i, c in Text(c.title).tag(i) }
                }
            }
            Text(o.detail + (o.applies == .respring ? " Takes effect after a respring." : " Takes effect after a reboot.")
                 + (o.experimental ? " Experimental." : ""))
                .font(.footnote).foregroundStyle(.secondary)
        }
    }
}
