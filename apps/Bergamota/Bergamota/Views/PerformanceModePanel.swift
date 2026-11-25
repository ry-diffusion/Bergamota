import SwiftUI

/// Performance mode: presets of the phone's performance switches, each switch under Advanced, applied over SSH.
struct PerformanceModePanel: View {
    @Environment(AppModel.self) private var model
    @State private var advanced = false

    var body: some View {
        let p = model.perf
        Panel(title: "Performance mode", symbol: "slider.horizontal.3") {
            HStack(spacing: 10) {
                ForEach(PerfCatalog.presets) { preset in
                    presetCard(preset, selected: p.activePreset?.id == preset.id) { p.choose(preset) }
                }
            }
            DisclosureGroup(isExpanded: $advanced) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(groups, id: \.self) { g in
                        Label12(text: g).padding(.top, 12).padding(.bottom, 4)
                        ForEach(PerfCatalog.options.filter { $0.group == g }) { optionRow($0) }
                    }
                }
            } label: {
                Text("Advanced").font(.callout.weight(.semibold)).foregroundStyle(Color.text)
            }
            .padding(.top, 4)
            footer(p)
        }
        .task(id: model.monitor.state) { await p.read() }
    }

    private var groups: [String] {
        var seen: [String] = []
        for o in PerfCatalog.options where !seen.contains(o.group) { seen.append(o.group) }
        return seen
    }

    private func presetCard(_ preset: PerfPreset, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(preset.title).font(BFont.display(16)).foregroundStyle(Color.text)
                    Spacer(minLength: 4)
                    if selected { Image(systemName: "checkmark.circle.fill").foregroundStyle(Color.leafText) }
                }
                Text(preset.detail).font(.caption).foregroundStyle(Color.muted).lineLimit(3).multilineTextAlignment(.leading)
            }
            .padding(12)
            .frame(maxWidth: .infinity, minHeight: 86, alignment: .topLeading)
            .background(selected ? Color.leafText.opacity(0.08) : Color.clear, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(selected ? Color.leafText : Color.rule))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func optionRow(_ o: PerfOption) -> some View {
        let p = model.perf
        let binding = Binding<Int>(get: { p.wanted[o.id] ?? 0 }, set: { p.wanted[o.id] = $0 })
        return HStack(alignment: .top, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(o.title).font(.callout.weight(.semibold)).foregroundStyle(Color.text)
                    Pill(text: o.applies == .respring ? "respring" : "reboot", tone: .ink, size: 9)
                    if o.experimental { Pill(text: "experimental", tone: .amber, size: 9) }
                    if p.wanted[o.id] != nil, p.wanted[o.id] != p.phone[o.id] { Pill(text: "changed", tone: .leaf, size: 9) }
                }
                Text(o.detail).font(.caption).foregroundStyle(Color.muted).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            Picker(o.title, selection: binding) {
                ForEach(Array(o.choices.enumerated()), id: \.offset) { i, c in Text(c.title).tag(i) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(width: o.choices.count > 2 ? 230 : 120)
        }
        .padding(.vertical, 8)
        .overlay(alignment: .bottom) { Rectangle().fill(Color.rule).frame(height: 1) }
    }

    @ViewBuilder
    private func footer(_ p: PerformanceMode) -> some View {
        HStack(spacing: 10) {
            Button { Task { await p.apply() } } label: {
                Text(p.changed.isEmpty ? "No changes" : "Apply \(p.changed.count) change\(p.changed.count == 1 ? "" : "s")")
            }
            .buttonStyle(.borderedProminent)
            .disabled(p.changed.isEmpty || p.applying)
            if p.pendingRespring {
                Button("Respring") { Task { await p.respring() } }
                    .help("Restart SpringBoard so its switches take effect (backboardd keeps running)")
            }
            Button { Task { await p.read() } } label: { Image(systemName: "arrow.clockwise") }
                .help("Read the switches from the phone again")
                .disabled(p.reading)
            if p.reading || p.applying { ProgressView().controlSize(.small) }
            Spacer()
        }
        .padding(.top, 6)
        if p.pendingReboot {
            Notice(text: "backboardd reads its switches when it starts: reboot the phone (Boot Queue) for them to take effect.",
                   symbol: "arrow.triangle.2.circlepath", color: .muted)
        }
        if let e = p.error { Notice(text: e) }
    }
}
