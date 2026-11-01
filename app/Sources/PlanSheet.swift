// Read-only preview of the default cleanup plan (the CLI's `clean --dry-run`).

import SwiftUI

struct PlanSheet: View {
    @Environment(\.dismiss) private var dismiss
    let report: Report
    let plan: Plan
    @State private var showSkipped = false

    var body: some View {
        let rules = plan.actions.filter { $0.source != "project" }
        let projects = plan.actions.filter { $0.source == "project" }
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Cleanup preview").font(.title2.weight(.semibold))
                Text("≈ \(Fmt.size(plan.totalBytes)) would be freed. This is a preview: nothing is changed. Cleaning from the app arrives in the next version.")
                    .foregroundColor(.secondary)
            }
            .padding(20)
            Divider()
            List {
                Section("Caches and tools (\(Fmt.size(rules.reduce(0) { $0 + $1.bytes })))") {
                    ForEach(rules) { a in ActionRow(report: report, action: a) }
                }
                Section("Project build folders: \(projects.count) projects untouched for 30+ days (\(Fmt.size(projects.reduce(0) { $0 + $1.bytes })))") {
                    ForEach(projects.prefix(40)) { a in
                        HStack {
                            Text(report.tilde(a.title.components(separatedBy: " (").first ?? a.title)).lineLimit(1).truncationMode(.middle)
                            Spacer()
                            Text(a.steps.compactMap { $0.path.map { ($0 as NSString).lastPathComponent } }.joined(separator: " "))
                                .foregroundColor(.secondary).font(.callout)
                            SizeText(bytes: a.bytes).frame(width: 80, alignment: .trailing)
                        }
                    }
                    if projects.count > 40 {
                        Text("… and \(projects.count - 40) smaller projects").foregroundColor(.secondary)
                    }
                }
                Section {
                    DisclosureGroup("Left out (\(plan.skipped.count))", isExpanded: $showSkipped) {
                        ForEach(plan.skipped.prefix(200)) { s in
                            HStack(alignment: .firstTextBaseline) {
                                Text(s.target.hasPrefix("/") ? report.tilde(s.target) : s.target).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(s.reason).foregroundColor(.secondary).font(.callout).lineLimit(1)
                                if s.bytes > 0 { SizeText(bytes: s.bytes).frame(width: 80, alignment: .trailing) }
                            }
                        }
                    }
                }
            }
            Divider()
            HStack {
                Text("Review-level items (Docker data, the Claude VM, model files) are never in the default plan.")
                    .font(.caption).foregroundColor(.secondary)
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            .padding(16)
        }
        .frame(width: 820, height: 620)
    }
}

struct ActionRow: View {
    let report: Report
    let action: PlanAction
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(action.title).font(.headline)
                SafetyBadge(safety: action.safety)
                Spacer()
                SizeText(bytes: action.bytes)
            }
            ForEach(action.steps, id: \.self) { s in
                HStack(spacing: 6) {
                    switch s.type {
                    case "run":
                        Text("runs").foregroundColor(.secondary)
                        CommandText(argv: s.argv ?? [])
                    case "delete-contents":
                        Text("empties \(report.tilde(s.path ?? ""))").foregroundColor(.secondary)
                    case "trash":
                        Text("moves to Trash: \(report.tilde(s.path ?? ""))").foregroundColor(.secondary)
                    default:
                        Text("deletes \(report.tilde(s.path ?? ""))").foregroundColor(.secondary)
                    }
                }
                .font(.callout)
            }
            ForEach(action.blockers ?? [], id: \.self) { b in
                Label(b, systemImage: "exclamationmark.triangle").foregroundColor(.orange).font(.callout)
            }
        }
        .padding(.vertical, 4)
    }
}
