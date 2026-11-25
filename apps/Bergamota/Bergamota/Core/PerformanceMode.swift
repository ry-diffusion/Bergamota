import Foundation
import Observation

/// The Performance page's mode panel: presets and the switches under them, read from and written to the phone over SSH.
@Observable
@MainActor
final class PerformanceMode {
    var phone: [String: Int] = [:]        // the choice index each option has on the phone (from the last read)
    var wanted: [String: Int] = [:]       // the choice index shown in the panel
    var reading = false
    var applying = false
    var error: String?
    var lastApplied: Date?
    var pendingRespring = false
    var pendingReboot = false

    @ObservationIgnored private let settings: AppSettings
    @ObservationIgnored private let monitor: PhoneMonitor
    init(settings: AppSettings, monitor: PhoneMonitor) { self.settings = settings; self.monitor = monitor }

    var changed: [PerfOption] { PerfCatalog.options.filter { wanted[$0.id] != nil && wanted[$0.id] != phone[$0.id] } }
    var activePreset: PerfPreset? { PerfCatalog.presets.first { p in p.choices.allSatisfy { wanted[$0.key] == $0.value } } }

    func choose(_ preset: PerfPreset) { for (k, v) in preset.choices { wanted[k] = v } }

    private var host: String { "root@\(settings.phoneIP)" }

    func read() async {
        guard monitor.state == .ios, let ssh = settings.ssh, !reading else { return }
        reading = true
        defer { reading = false }
        // one line per option: name, then its contents with newlines as |, or ABSENT
        let names = PerfCatalog.options.map { shellQuote($0.id) }.joined(separator: " ")
        let cmd = "cd \(PerfCatalog.dir) && for f in \(names); do if [ -e \"$f\" ]; then printf '%s=' \"$f\"; head -c 200 \"$f\" | tr '\\n' '|'; echo; else echo \"$f=ABSENT\"; fi; done"
        let r = await Shell.run(ssh, Shell.sshOptions + [host, cmd], timeout: 15)
        guard r.ok else { error = r.launchError ?? (r.timedOut ? "ssh timed out" : r.stderr.trimmingCharacters(in: .whitespacesAndNewlines)); return }
        error = nil
        var found: [String: Int] = [:]
        for line in r.stdout.split(separator: "\n") {
            guard let eq = line.firstIndex(of: "=") else { continue }
            let name = String(line[..<eq]), value = String(line[line.index(after: eq)...])
            guard let o = PerfCatalog.options.first(where: { $0.id == name }) else { continue }
            found[name] = PerfCatalog.choiceIndex(o, contents: value == "ABSENT" ? nil : value.replacingOccurrences(of: "|", with: "\n"))
        }
        phone = found
        for (k, v) in found where wanted[k] == nil { wanted[k] = v }
    }

    func apply() async {
        guard monitor.state == .ios, let ssh = settings.ssh, !applying else { return }
        let todo = changed
        guard !todo.isEmpty else { return }
        applying = true
        defer { applying = false }
        let r = await Shell.run(ssh, Shell.sshOptions + [host, PerfCatalog.commands(for: todo, wanted: wanted).joined(separator: " && ")], timeout: 15)
        guard r.ok else { error = r.launchError ?? r.stderr.trimmingCharacters(in: .whitespacesAndNewlines); return }
        DebugLog.write("performance mode: " + todo.map { "\($0.id)=\(wanted[$0.id] ?? -1)" }.joined(separator: " "))
        pendingRespring = pendingRespring || todo.contains { $0.applies == .respring }
        pendingReboot = pendingReboot || todo.contains { $0.applies == .reboot }
        lastApplied = Date()
        await read()
    }

    /// Restarts SpringBoard (backboardd keeps running), so its switches take effect.
    func respring() async {
        guard monitor.state == .ios, let ssh = settings.ssh else { return }
        let cmd = "p=$(ps -A -o pid,comm | grep '[S]pringBoard.app/SpringBoard' | cut -c1-6 | tr -d ' '); [ -n \"$p\" ] && kill $p"
        let r = await Shell.run(ssh, Shell.sshOptions + [host, cmd], timeout: 15)
        if r.ok { pendingRespring = false } else { error = r.stderr.trimmingCharacters(in: .whitespacesAndNewlines) }
    }
}
