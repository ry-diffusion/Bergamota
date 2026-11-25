// Tests for Core/PerfCatalog.swift: file state -> choice, choice -> shell lines. Run: Tests/run-perf-tests.sh
import Foundation

var failures = 0
func check(_ ok: Bool, _ what: String, line: Int = #line) {
    if ok { print("ok   \(what)") } else { print("FAIL \(what) (line \(line))"); failures += 1 }
}
func opt(_ id: String) -> PerfOption { PerfCatalog.options.first { $0.id == id }! }

let keep = opt("m2v-sbkeepicons"), gpu = opt("bgmtl-bb.env"), cut = opt("m2v-midcut")
check(keep.isFlag && !gpu.isFlag && !cut.isFlag, "flags vs files with contents")
check(PerfCatalog.choiceIndex(keep, contents: nil) == 0 && PerfCatalog.choiceIndex(keep, contents: "") == 1, "flag: absent off, empty on")
check(PerfCatalog.choiceIndex(keep, contents: "anything\n") == 1, "flag: a file with contents is on")
check(PerfCatalog.choiceIndex(gpu, contents: nil) == 0 && PerfCatalog.choiceIndex(gpu, contents: "") == 0, "GPU: absent or empty is tiled")
check(PerfCatalog.choiceIndex(gpu, contents: "TU_DEBUG=sysmem\n") == 1, "GPU: sysmem")
check(PerfCatalog.choiceIndex(gpu, contents: "TU_AUTOTUNE_ALGO=profiled") == 2, "GPU: profiled without a newline")
check(PerfCatalog.choiceIndex(cut, contents: "always\n40\n\n") == 1 && PerfCatalog.choiceIndex(cut, contents: nil) == 0, "midcut")
check(PerfCatalog.commands(for: [keep], wanted: [keep.id: 0]) == ["rm -f /private/var/tmp/m2v-sbkeepicons"], "off removes the file")
check(PerfCatalog.commands(for: [keep], wanted: [keep.id: 1]) == ["printf %s '' > /private/var/tmp/m2v-sbkeepicons"], "on writes an empty file")
check(PerfCatalog.commands(for: [gpu], wanted: [gpu.id: 1]) == ["printf %s 'TU_DEBUG=sysmem\n' > /private/var/tmp/bgmtl-bb.env"], "contents written as given")
check(PerfCatalog.commands(for: [gpu], wanted: [:]).isEmpty && PerfCatalog.commands(for: [gpu], wanted: [gpu.id: 9]).isEmpty, "unknown choice: nothing")
for p in PerfCatalog.presets {
    check(p.choices.allSatisfy { k, v in PerfCatalog.options.first { $0.id == k }.map { $0.choices.indices.contains(v) } ?? false }, "preset \(p.id): known options, valid choices")
}
check(Set(PerfCatalog.options.map(\.id)).count == PerfCatalog.options.count, "option ids unique")
print(failures == 0 ? "all passed" : "\(failures) failed")
exit(failures == 0 ? 0 : 1)
