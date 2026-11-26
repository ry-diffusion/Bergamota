import Foundation

/// One performance switch on the phone. Every one is a file in /private/var/tmp that libcrimson (Crimson's
/// ios/crimson.mm and ios/metal2/NativeBridgeMetalFrontend.mm) or the Crimson inject reads once, when the process
/// that uses it starts: SpringBoard's on a respring, backboardd's on a reboot.
struct PerfOption: Identifiable, Hashable {
    enum Applies: String { case respring = "respring", reboot = "reboot" }
    struct Choice: Hashable { let title: String; let content: String? }   // content nil: the file is absent

    let id: String                  // the file name in /private/var/tmp
    let title: String
    let detail: String
    let group: String
    let applies: Applies
    let choices: [Choice]           // two for an on/off flag (absent, present), more for a file with contents
    var experimental = false

    static func flag(_ id: String, _ title: String, _ detail: String, group: String, applies: Applies,
                     experimental: Bool = false) -> PerfOption {
        PerfOption(id: id, title: title, detail: detail, group: group, applies: applies,
                   choices: [Choice(title: "Off", content: nil), Choice(title: "On", content: "")], experimental: experimental)
    }
    var isFlag: Bool { choices.count == 2 && choices[0].content == nil && choices[1].content == "" }
    var path: String { PerfCatalog.dir + "/" + id }
}

/// A named set of choices (option id -> choice index); options it leaves out keep their current choice.
struct PerfPreset: Identifiable, Hashable {
    let id: String
    let title: String
    let detail: String
    let choices: [String: Int]
}

/// The phone's performance switches and presets, and how a choice maps to a file (pure: Tests/perf-tests.swift).
enum PerfCatalog {
    static let dir = "/private/var/tmp"

    static let options: [PerfOption] = [
        .flag("crimson-sbkeepicons", "Keep home pages built",
              "SpringBoard keeps every home page, the Today view and their widgets built while you swipe. Swiping between pages rebuilt them every time: 40-43 fps with a 0.4 s stall per swipe, 57-60 without.",
              group: "SpringBoard", applies: .respring),
        .flag("crimson-blurcap", "Half-resolution blurs",
              "SpringBoard asks for no blur finer than half resolution (glass in the App Library, folders, Control Center). Measured: App Library GPU 21.0-21.3 -> 20.5 ms a frame, so little gain.",
              group: "SpringBoard", applies: .respring, experimental: true),
        PerfOption(id: "crimson-bb.env", title: "GPU rendering",
                   detail: "How Turnip renders backboardd's passes. Tiled (GMEM) gave the steadiest page drags; direct (sysmem) was the old setting; profiled times both and was slower here.",
                   group: "backboardd", applies: .reboot,
                   choices: [PerfOption.Choice(title: "Tiled", content: ""),
                             PerfOption.Choice(title: "Direct", content: "TU_DEBUG=sysmem\n"),
                             PerfOption.Choice(title: "Profiled", content: "TU_AUTOTUNE_ALGO=profiled\n")]),
        .flag("crimson-batch", "Batched encoding", "Encoder work is recorded in batches on worker threads instead of call by call.",
              group: "backboardd", applies: .reboot),
        .flag("crimson-defer", "Deferred encoding", "CoreAnimation's Metal calls are queued and run off its thread.",
              group: "backboardd", applies: .reboot),
        .flag("crimson-async-commit", "Asynchronous commit", "A command buffer's commit returns before its work is submitted (needs deferred encoding).",
              group: "backboardd", applies: .reboot),
        .flag("crimson-bb-nowait", "No scheduled waits", "backboardd does not block on waitUntilScheduled.",
              group: "backboardd", applies: .reboot),
        .flag("crimson-cut", "Early submit", "Passes are submitted to the GPU as they end instead of all at the commit.",
              group: "backboardd", applies: .reboot),
        PerfOption(id: "crimson-midcut", title: "Mid-pass cut",
                   detail: "Long passes are split every 40 draws so the GPU starts sooner. Cheap only with Direct rendering.",
                   group: "backboardd", applies: .reboot,
                   choices: [PerfOption.Choice(title: "Off", content: nil), PerfOption.Choice(title: "Every 40", content: "always\n40\n")]),
        .flag("crimson-hud", "On-screen HUD", "The fps/GPU overlay in the top-left corner.", group: "Diagnostics", applies: .reboot),
        .flag("crimson-fps", "fps log", "A per-second fps line in crimson.log.", group: "Diagnostics", applies: .reboot),
        .flag("crimson-telemetry", "Telemetry", "Timing summaries in crimson.log.", group: "Diagnostics", applies: .reboot),
        .flag("crimson-cpumeter", "CPU meter", "Per-batch CPU time in the fps line (two system calls a batch).", group: "Diagnostics", applies: .reboot),
    ]

    private static let pipelineOn = ["crimson-batch": 1, "crimson-defer": 1, "crimson-async-commit": 1, "crimson-bb-nowait": 1, "crimson-cut": 1]

    static let presets: [PerfPreset] = [
        PerfPreset(id: "smooth", title: "Smooth",
                   detail: "Everything measured to help: pages kept built, tiled GPU rendering, the full encoding pipeline.",
                   choices: pipelineOn.merging(["crimson-sbkeepicons": 1, "crimson-bb.env": 0, "crimson-midcut": 0, "crimson-blurcap": 0]) { $1 }),
        PerfPreset(id: "balanced", title: "Balanced",
                   detail: "As Smooth, but SpringBoard manages pages and widgets itself (less memory; Today widgets keep their normal lifecycle).",
                   choices: pipelineOn.merging(["crimson-sbkeepicons": 0, "crimson-bb.env": 0, "crimson-midcut": 0, "crimson-blurcap": 0]) { $1 }),
        PerfPreset(id: "safe", title: "Safe",
                   detail: "No SpringBoard changes and the plainest pipeline: for checking whether an optimization causes a problem.",
                   choices: ["crimson-sbkeepicons": 0, "crimson-blurcap": 0, "crimson-bb.env": 0, "crimson-midcut": 0,
                             "crimson-batch": 0, "crimson-defer": 0, "crimson-async-commit": 0, "crimson-bb-nowait": 0, "crimson-cut": 0]),
    ]

    /// The choice index of an option from the file's state: nil (absent) or its contents.
    static func choiceIndex(_ o: PerfOption, contents: String?) -> Int {
        if let i = o.choices.firstIndex(where: { $0.content == contents }) { return i }
        if contents != nil, o.isFlag { return 1 }                          // a flag file with anything in it is on
        if contents == nil, let i = o.choices.firstIndex(where: { $0.content == "" }) { return i }   // no file = empty file
        let trimmed = contents?.trimmingCharacters(in: .whitespacesAndNewlines)
        return o.choices.firstIndex { $0.content?.trimmingCharacters(in: .whitespacesAndNewlines) == trimmed } ?? 0
    }

    /// The shell lines that make the phone's files match `wanted` for the changed options.
    static func commands(for options: [PerfOption], wanted: [String: Int]) -> [String] {
        options.compactMap { o in
            guard let i = wanted[o.id], o.choices.indices.contains(i) else { return nil }
            guard let content = o.choices[i].content else { return "rm -f \(shellQuote(o.path))" }
            // owned by mobile, so the phone's own Bergamota app (which runs as mobile) can change or remove it in the sticky /private/var/tmp
            return "printf %s \(shellQuote(content)) > \(shellQuote(o.path)) && chown mobile:mobile \(shellQuote(o.path))"
        }
    }
}
