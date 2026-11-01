// Cleaning, always with a person in the loop (PLAN.md §5):
// nothing is preselected; the person ticks items, reviews the exact steps, and
// confirms. The Rust core re-checks every step on disk before running it.

import AppKit
import SwiftUI

struct CleanStepResult: Decodable, Hashable {
    let step: Step
    let ok: Bool
    let skipped: Bool
    let message: String
    let beforeBytes: UInt64
    let afterBytes: UInt64
    let freedBytes: UInt64
}

struct CleanActionResult: Decodable, Identifiable {
    var id: String { target }
    let target: String
    let title: String
    let freedBytes: UInt64
    let steps: [CleanStepResult]
}

struct CleanResult: Decodable {
    let actions: [CleanActionResult]
    let freedBytes: UInt64
    let freeBefore: UInt64?
    let freeAfter: UInt64?
    let logPath: String
}

final class Cleaner: ObservableObject {
    enum Stage {
        case choose
        case confirm
        case running(index: Int, total: Int, what: String)
        case done(CleanResult, skipped: [Skipped])
        case failed(String)
    }

    @Published var candidates: Plan?
    @Published var candidatesError: String?
    @Published var selected: Set<String> = []
    @Published var stage: Stage = .choose
    @Published var showSheet = false

    var isRunning: Bool {
        if case .running = stage { return true }
        return false
    }

    var selectedActions: [PlanAction] {
        (candidates?.actions ?? []).filter { selected.contains($0.target) }
    }

    var selectedBytes: UInt64 { selectedActions.reduce(0) { $0 + $1.bytes } }

    static func decoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }

    /// Everything that could be cleaned, from the last scan. Nothing selected.
    func loadCandidates() {
        selected = []
        let raw = "{\"candidates\":true}".withCString { ss_plan($0) }
        defer { ss_free(raw) }
        guard let raw = raw else { return }
        let data = Data(String(cString: raw).utf8)
        struct Err: Decodable { let error: String }
        if let e = try? Cleaner.decoder().decode(Err.self, from: data) {
            candidates = nil
            candidatesError = e.error
            return
        }
        candidates = try? Cleaner.decoder().decode(Plan.self, from: data)
        candidatesError = candidates == nil ? "couldn't read the cleanup list" : nil
    }

    /// Open the sheet: on the list, or straight at the confirmation for targets
    /// the person just picked elsewhere (a card's Clean button, table rows).
    func open(with targets: [String] = []) {
        guard !isRunning else { showSheet = true; return }
        var known = Set((candidates?.actions ?? []).map { $0.target })
        // Targets that aren't in the candidate list (folders picked in the Folders
        // screen): plan them now and add them, so they can be reviewed.
        let extra = targets.filter { !known.contains($0) }
        if !extra.isEmpty, let p = Cleaner.plan(targets: extra) {
            var list = candidates?.actions ?? []
            list.append(contentsOf: p.actions.filter { !known.contains($0.target) })
            candidates = Plan(actions: list, skipped: candidates?.skipped ?? [], totalBytes: candidates?.totalBytes ?? 0)
            known = Set(list.map { $0.target })
            if p.actions.isEmpty, let s = p.skipped.first {
                stage = .failed("Can't clean \(s.target): \(s.reason).")
                showSheet = true
                return
            }
        }
        selected = Set(targets.filter { known.contains($0) })
        stage = selected.isEmpty ? .choose : .confirm
        showSheet = true
    }

    /// Plan exact targets (never changes anything).
    static func plan(targets: [String]) -> Plan? {
        guard let d = try? JSONSerialization.data(withJSONObject: ["targets": targets]),
              let req = String(data: d, encoding: .utf8) else { return nil }
        let raw = req.withCString { ss_plan($0) }
        defer { ss_free(raw) }
        guard let raw = raw else { return nil }
        return try? Cleaner.decoder().decode(Plan.self, from: Data(String(cString: raw).utf8))
    }

    /// Apps that must be quit, found by bundle id (folder names like
    /// com.spotify.client), which the core can't see in the process list.
    static func runningOwners(_ actions: [PlanAction]) -> [String] {
        let running = NSWorkspace.shared.runningApplications
        var out: [String] = []
        for a in actions where a.source == "folder" || a.source == "chrome-ai-model" {
            for comp in a.title.split(separator: "/").map(String.init) {
                if let app = running.first(where: { $0.bundleIdentifier == comp }) {
                    out.append("quit \(app.localizedName ?? comp) first (\(comp))")
                }
            }
        }
        return out
    }

    func isCandidate(_ target: String) -> Bool {
        (candidates?.actions ?? []).contains { $0.target == target }
    }

    private final class Sink {
        weak var cleaner: Cleaner?
        init(_ c: Cleaner) { cleaner = c }
    }

    /// Runs only after the person pressed the confirm button.
    func run(then rescan: @escaping () -> Void) {
        let targets = Array(selected)
        guard !targets.isEmpty, !isRunning else { return }
        stage = .running(index: 0, total: selectedActions.reduce(0) { $0 + $1.steps.count }, what: "starting")
        guard let reqData = try? JSONSerialization.data(withJSONObject: ["targets": targets]),
              let req = String(data: reqData, encoding: .utf8) else { return }
        let sink = Unmanaged.passRetained(Sink(self))
        let activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiated], reason: "Cleaning")
        DispatchQueue.global(qos: .userInitiated).async {
            let cb: ss_clean_cb = { ctx, i, n, what in
                guard let ctx = ctx else { return }
                let sink = Unmanaged<Sink>.fromOpaque(ctx).takeUnretainedValue()
                let text = what.map { String(cString: $0) } ?? ""
                DispatchQueue.main.async {
                    sink.cleaner?.stage = .running(index: Int(i), total: Int(n), what: text)
                }
            }
            let raw = req.withCString { ss_clean($0, cb, sink.toOpaque()) }
            let json = raw.map { String(cString: $0) } ?? "{\"error\":\"no result\"}"
            ss_free(raw)
            struct Payload: Decodable {
                let result: CleanResult?
                let skipped: [Skipped]?
                let error: String?
            }
            let p = try? Cleaner.decoder().decode(Payload.self, from: Data(json.utf8))
            DispatchQueue.main.async {
                sink.release()
                ProcessInfo.processInfo.endActivity(activity)
                if let r = p?.result {
                    self.stage = .done(r, skipped: p?.skipped ?? [])
                } else {
                    self.stage = .failed(p?.error ?? "couldn't read the result")
                }
                self.selected = []
                rescan()
            }
        }
    }
}

// MARK: - Sheet

struct CleanupSheet: View {
    @EnvironmentObject var cleaner: Cleaner
    @EnvironmentObject var scanner: Scanner
    @Environment(\.dismiss) private var dismiss
    let report: Report

    var body: some View {
        VStack(spacing: 0) {
            switch cleaner.stage {
            case .choose: ChooseView(report: report)
            case .confirm: ConfirmView(report: report)
            case .running(let i, let n, let what): RunningView(index: i, total: n, what: what, report: report)
            case .done(let r, let skipped): DoneView(result: r, skipped: skipped, report: report)
            case .failed(let msg):
                VStack(spacing: 12) {
                    Text("Cleaning didn't run").font(.title2)
                    Text(msg).foregroundColor(.secondary)
                    Button("Close") { cleaner.showSheet = false }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(width: 880, height: 660)
        .interactiveDismissDisabled(cleaner.isRunning)
    }
}

struct ChooseView: View {
    @EnvironmentObject var cleaner: Cleaner
    let report: Report

    var body: some View {
        let all = cleaner.candidates?.actions ?? []
        let caches = all.filter { $0.source != "project" && $0.source != "large-file" && $0.safety != .review }
        let review = all.filter { $0.source != "project" && $0.source != "large-file" && $0.safety == .review }
        let projects = all.filter { $0.source == "project" }.sorted { $0.bytes > $1.bytes }
        let files = all.filter { $0.source == "large-file" }
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Choose what to clean").font(.title2.weight(.semibold))
                Text("Nothing is selected. Tick only what you want gone; you'll see every exact step before anything happens.")
                    .foregroundColor(.secondary)
            }
            .padding(20)
            Divider()
            if let e = cleaner.candidatesError {
                Text("Cleaning needs a scan in this session (⌘R). \(e)").foregroundColor(.secondary).padding(20)
                Spacer()
            } else {
                List {
                    Section("Caches and tools") {
                        ForEach(caches) { a in CandidateRow(report: report, action: a) }
                    }
                    if !review.isEmpty {
                        Section("Needs a look first") {
                            ForEach(review) { a in CandidateRow(report: report, action: a) }
                        }
                    }
                    Section("Project build folders (\(projects.count) projects not active in the last 14 days)") {
                        ForEach(projects) { a in CandidateRow(report: report, action: a) }
                    }
                    if !files.isEmpty {
                        Section("Large files (moved to the Trash)") {
                            ForEach(files) { a in CandidateRow(report: report, action: a) }
                        }
                    }
                }
            }
            Divider()
            HStack {
                Button("Tick suggested") {
                    for a in all where a.suggested { cleaner.selected.insert(a.target) }
                }
                .help("Ticks caches, rebuildable data and projects untouched for 30+ days. You still review before anything runs.")
                Button("Untick all") { cleaner.selected = [] }
                    .disabled(cleaner.selected.isEmpty)
                Spacer()
                Text("\(cleaner.selected.count) selected · \(Fmt.size(cleaner.selectedBytes))")
                    .font(.body.monospacedDigit()).foregroundColor(.secondary)
                Button("Cancel") { cleaner.showSheet = false }
                    .keyboardShortcut(.cancelAction)
                Button("Review…") { cleaner.stage = .confirm }
                    .keyboardShortcut(.defaultAction)
                    .disabled(cleaner.selected.isEmpty)
            }
            .padding(16)
        }
    }
}

struct CandidateRow: View {
    @EnvironmentObject var cleaner: Cleaner
    let report: Report
    let action: PlanAction

    var body: some View {
        let on = Binding(
            get: { cleaner.selected.contains(action.target) },
            set: { v in if v { cleaner.selected.insert(action.target) } else { cleaner.selected.remove(action.target) } })
        Toggle(isOn: on) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        Text(report.tilde(action.title)).lineLimit(1).truncationMode(.middle)
                        SafetyBadge(safety: action.safety)
                    }
                    Text(summary).font(.caption).foregroundColor(.secondary).lineLimit(1).truncationMode(.middle)
                    ForEach(action.blockers ?? [], id: \.self) { b in
                        Label(b, systemImage: "exclamationmark.triangle").font(.caption).foregroundColor(.orange)
                    }
                }
                Spacer()
                SizeText(bytes: action.bytes).foregroundColor(.secondary)
            }
        }
        .toggleStyle(.checkbox)
    }

    var summary: String {
        action.steps.map { s in stepText(s, report: report, short: true) }.joined(separator: " · ")
    }
}

func stepText(_ s: Step, report: Report, short: Bool = false) -> String {
    let p = report.tilde(s.path ?? "")
    switch s.type {
    case "run": return "runs " + Actions.shell((s.argv ?? []).enumerated().map { $0.offset == 0 ? ($0.element as NSString).lastPathComponent : $0.element })
    case "delete-contents": return "deletes everything inside \(p)"
    case "trash": return "moves \(short ? (p as NSString).lastPathComponent : p) to the Trash"
    default: return "deletes \(short ? (p as NSString).lastPathComponent : p)"
    }
}

struct ConfirmView: View {
    @EnvironmentObject var cleaner: Cleaner
    @EnvironmentObject var scanner: Scanner
    let report: Report

    var body: some View {
        let actions = cleaner.selectedActions
        let permanent = actions.contains { $0.steps.contains { $0.type == "delete-dir" || $0.type == "delete-contents" || $0.type == "run" } }
        let blocked = actions.filter { !($0.blockers ?? []).isEmpty }
        let owners = Cleaner.runningOwners(actions)
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Review before cleaning").font(.title2.weight(.semibold))
                Text("SpaceSense will do exactly these \(actions.reduce(0) { $0 + $1.steps.count }) steps and nothing else. About \(Fmt.size(cleaner.selectedBytes)).")
                if permanent {
                    Label("Deleted caches and build folders can't be put back; your tools download or rebuild them when needed. Items moved to the Trash can be put back until you empty it.", systemImage: "info.circle")
                        .font(.callout).foregroundColor(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(blocked) { a in
                    Label("\(report.tilde(a.title)): \((a.blockers ?? []).joined(separator: ", ")). It will be skipped unless you do that before cleaning.", systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundColor(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(owners, id: \.self) { o in
                    Label("To clean this app's folder, \(o). Then press Check again.", systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundColor(.orange)
                }
            }
            .padding(20)
            Divider()
            List {
                ForEach(actions) { a in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(report.tilde(a.title)).font(.headline).lineLimit(1).truncationMode(.middle)
                            SafetyBadge(safety: a.safety)
                            Spacer()
                            SizeText(bytes: a.bytes)
                        }
                        ForEach(a.steps, id: \.self) { s in
                            Text("• " + stepText(s, report: report))
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                                .lineLimit(2)
                        }
                        ForEach(a.warnings ?? [], id: \.self) { w in
                            Text(w).font(.caption).foregroundColor(.secondary)
                        }
                    }
                    .padding(.vertical, 3)
                }
            }
            Divider()
            HStack {
                Text("Every step is checked again on disk right before it runs, and logged.")
                    .font(.caption).foregroundColor(.secondary)
                Spacer()
                Button("Back") { cleaner.stage = .choose }
                    .keyboardShortcut(.cancelAction)
                if !owners.isEmpty {
                    Button("Check again") { cleaner.objectWillChange.send() }
                }
                Button("Clean \(actions.count) item\(actions.count == 1 ? "" : "s")") {
                    cleaner.run { scanner.start(root: LaunchOptions.current.root) }
                }
                .buttonStyle(.borderedProminent)
                .tint(.red)
                .disabled(actions.isEmpty || !owners.isEmpty)
            }
            .padding(16)
        }
    }
}

struct RunningView: View {
    let index: Int
    let total: Int
    let what: String
    let report: Report
    var body: some View {
        VStack(spacing: 16) {
            ProgressView(value: Double(min(index, total)), total: Double(max(total, 1)))
                .frame(width: 420)
            Text("Step \(min(index + 1, max(total, 1))) of \(max(total, 1))").font(.headline)
            Text(report.tilde(what)).foregroundColor(.secondary).lineLimit(2).truncationMode(.middle)
                .frame(maxWidth: 600)
            Text("Large caches can take a few minutes. Please keep SpaceSense open.")
                .font(.callout).foregroundColor(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct DoneView: View {
    @EnvironmentObject var cleaner: Cleaner
    let result: CleanResult
    let skipped: [Skipped]
    let report: Report

    var body: some View {
        let trashed = result.actions.flatMap { $0.steps }.filter { $0.ok && $0.step.type == "trash" }
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Freed \(Fmt.size(result.freedBytes))").font(.system(size: 28, weight: .semibold))
                if let b = result.freeBefore, let a = result.freeAfter {
                    Text("Free space: \(Fmt.size(b)) → \(Fmt.size(a)). Measured on disk, not estimated.")
                        .foregroundColor(.secondary)
                }
                if !trashed.isEmpty {
                    Text("\(trashed.count) item(s) are in the Trash: their space returns when you empty it.")
                        .font(.callout).foregroundColor(.secondary)
                }
            }
            .padding(20)
            Divider()
            List {
                ForEach(result.actions) { a in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack {
                            Text(report.tilde(a.title)).font(.headline).lineLimit(1).truncationMode(.middle)
                            Spacer()
                            SizeText(bytes: a.freedBytes)
                        }
                        ForEach(a.steps, id: \.self) { s in
                            Label(s.message, systemImage: s.ok ? "checkmark.circle" : (s.skipped ? "minus.circle" : "xmark.circle"))
                                .foregroundColor(s.ok ? .green : (s.skipped ? .orange : .red))
                                .font(.callout)
                                .lineLimit(3)
                        }
                    }
                }
                ForEach(skipped) { s in
                    Label("\(report.tilde(s.target)): \(s.reason)", systemImage: "minus.circle")
                        .foregroundColor(.orange).font(.callout)
                }
            }
            Divider()
            HStack {
                Button("Show log") { Actions.reveal(result.logPath) }
                Spacer()
                Text("The numbers update in the background; you can keep working.").font(.caption).foregroundColor(.secondary)
                Button("Done") { cleaner.showSheet = false }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(16)
        }
    }
}
