// Developer and Apps & System screens: one card per catalog rule.

import SwiftUI

struct RulesView: View {
    let report: Report
    let screen: Screen

    var subtitle: String {
        switch screen {
        case .developer: return "Package caches, Docker, Xcode, AI models, browser test runtimes and toolchains."
        default: return "App caches, app data, logs, the Trash and backups."
        }
    }

    var body: some View {
        let rules = report.rules(for: screen)
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                ScreenHeader(title: screen.rawValue, subtitle: subtitle, total: rules.reduce(0) { $0 + $1.size })
                    .padding(.bottom, 6)
                ForEach(rules) { r in
                    RuleCard(report: report, rule: r)
                }
                Text("Every path and command comes from SpaceSense's open catalog. Cleaning arrives in the next version; until then you can run the commands shown yourself.")
                    .font(.caption).foregroundColor(.secondary)
                    .padding(.top, 6)
            }
            .padding(24)
            .frame(maxWidth: 1000, alignment: .leading)
        }
    }
}

struct RuleCard: View {
    @EnvironmentObject var cleaner: Cleaner
    let report: Report
    let rule: RuleReport
    @State private var expanded = false

    var flags: [(String, Color, String)] {
        var f: [(String, Color, String)] = []
        if rule.leftover { f.append(("App uninstalled: leftover data", .orange, "trash")) }
        if rule.toolMissing && !rule.leftover, let t = rule.rule.tool { f.append(("\(t) not installed", .secondary, "questionmark.circle")) }
        for a in rule.runningApps ?? [] { f.append(("\(a) is running", .orange, "exclamationmark.circle")) }
        if let n = rule.note { f.append((n, .secondary, "info.circle")) }
        return f
    }

    var children: [PathSize] {
        ((rule.details ?? []) + (rule.items ?? [])).filter { $0.size >= 10_000_000 }
    }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(rule.name).font(.title3.weight(.semibold))
                    SafetyBadge(safety: rule.safety)
                    Spacer()
                    SizeText(bytes: rule.size, font: .title3.weight(.semibold))
                        .foregroundColor(rule.size == 0 ? .secondary : .primary)
                    if cleaner.isCandidate(rule.id) {
                        Button("Clean…") { cleaner.open(with: [rule.id]) }
                            .help("Shows exactly what will happen; nothing runs until you confirm.")
                    }
                }
                if !flags.isEmpty {
                    HStack(spacing: 6) {
                        ForEach(flags, id: \.0) { f in Chip(text: f.0, color: f.1, symbol: f.2) }
                    }
                }
                Text(rule.leftover ? (rule.rule.leftoverExplanation ?? rule.rule.explanation) : rule.rule.explanation)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text("Comes back:").foregroundColor(.secondary)
                    Text(rule.leftover ? "No. The app that made this is uninstalled, so nothing will recreate it." : rule.rule.comesBack)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .font(.callout)
                cleanup.font(.callout)
                ForEach(rule.paths, id: \.path) { p in
                    PathLine(path: p.path, display: report.tilde(p.path))
                }
                if !children.isEmpty {
                    DisclosureGroup(isExpanded: $expanded) {
                        VStack(spacing: 6) {
                            ForEach(children, id: \.path) { c in
                                HStack {
                                    Text(report.tilde(c.path)).lineLimit(1).truncationMode(.middle)
                                    Spacer()
                                    SizeText(bytes: c.size).foregroundColor(.secondary)
                                    Button { Actions.reveal(c.path) } label: { Image(systemName: "arrow.right.circle") }
                                        .buttonStyle(.borderless)
                                }
                                .font(.callout)
                            }
                        }
                        .padding(.top, 4)
                    } label: {
                        Text(rule.rule.generic ? "Biggest items (\(children.count))" : "What's inside").font(.callout)
                    }
                }
            }
        }
    }

    @ViewBuilder var cleanup: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text("Cleanup:").foregroundColor(.secondary)
            if rule.safety == .neverAuto {
                Text("SpaceSense never cleans this; it only shows the size.")
            } else if let cmd = rule.rule.officialCmd, !rule.leftover, rule.tool != nil {
                CommandText(argv: cmd)
                if let d = rule.rule.dryRunCmd {
                    Text("preview:").foregroundColor(.secondary)
                    CommandText(argv: d)
                }
            } else {
                Text(fallbackText)
            }
        }
    }

    var fallbackText: String {
        let fb = rule.leftover ? (rule.rule.leftoverFallback ?? rule.rule.fallback) : rule.rule.fallback
        switch fb {
        case "delete-contents": return rule.rule.officialCmd != nil ? "Delete the folder's contents (the official tool isn't installed)." : "Delete the folder's contents."
        case "delete-dir": return "Delete the folder."
        case "trash": return "Move to the Trash (recoverable)."
        default:
            if let t = rule.rule.tool, rule.rule.officialCmd != nil { return "Needs \(t), which isn't installed." }
            if rule.rule.generic { return "Per app, while that app is quit." }
            return "No automatic cleanup. Reveal it in Finder to review what's there."
        }
    }
}
