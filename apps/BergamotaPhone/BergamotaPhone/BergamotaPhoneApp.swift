import SwiftUI
import Darwin

/// Bergamota on the phone: the Mac app's Performance mode (same PerfCatalog), writing the switch files directly.
/// Runs as mobile without a sandbox (entitlements.plist); the Mac app hands each switch file to mobile, so this app
/// can change or remove it in the sticky /private/var/tmp.
@main
struct BergamotaPhoneApp: App {
    var body: some Scene { WindowGroup { PerformanceScreen() } }
}

/// PerfCatalog.commands quotes its arguments with this (the Mac app has it in Core/Shell.swift).
func shellQuote(_ s: String) -> String {
    if !s.isEmpty && s.allSatisfy({ $0.isLetter || $0.isNumber || "-_./=:@%+,".contains($0) }) { return s }
    return "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
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

    func read() {
        var found: [String: Int] = [:]
        for o in PerfCatalog.options {
            let contents = FileManager.default.fileExists(atPath: o.path) ? ((try? String(contentsOfFile: o.path, encoding: .utf8)) ?? "") : nil
            found[o.id] = PerfCatalog.choiceIndex(o, contents: contents)
        }
        phone = found
        for (k, v) in found where wanted[k] == nil { wanted[k] = v }
    }

    func apply() {
        error = nil
        for o in changed {
            guard let i = wanted[o.id], o.choices.indices.contains(i) else { continue }
            do {
                if let content = o.choices[i].content {
                    try content.write(toFile: o.path, atomically: false, encoding: .utf8)
                } else if FileManager.default.fileExists(atPath: o.path) {
                    try FileManager.default.removeItem(atPath: o.path)
                }
                if o.applies == .respring { pendingRespring = true } else { pendingReboot = true }
            } catch {
                self.error = "\(o.title): \(error.localizedDescription) (if the file belongs to root, apply once from Bergamota on the Mac)"
            }
        }
        read()
    }

    /// SpringBoard runs as mobile like this app: a SIGTERM restarts it (backboardd keeps running).
    func respring() {
        var mib = [CTL_KERN, KERN_PROC, KERN_PROC_ALL]
        var size = 0
        guard sysctl(&mib, 3, nil, &size, nil, 0) == 0 else { error = "process list: errno \(errno)"; return }
        var procs = [kinfo_proc](repeating: kinfo_proc(), count: size / MemoryLayout<kinfo_proc>.stride + 16)
        size = procs.count * MemoryLayout<kinfo_proc>.stride
        guard sysctl(&mib, 3, &procs, &size, nil, 0) == 0 else { error = "process list: errno \(errno)"; return }
        for p in procs.prefix(size / MemoryLayout<kinfo_proc>.stride) {
            let name = withUnsafeBytes(of: p.kp_proc.p_comm) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
            if name == "SpringBoard" { kill(p.kp_proc.p_pid, SIGTERM); pendingRespring = false; return }
        }
        error = "SpringBoard not found"
    }
}

struct PerformanceScreen: View {
    @State private var s = PhoneSwitches()

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
                if s.pendingReboot {
                    Section { Label("backboardd reads its switches when it starts: reboot the phone from Bergamota on the Mac.", systemImage: "arrow.triangle.2.circlepath") }
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
