// Git worktrees (core/src/worktrees.rs): size, build folders and whether each
// still holds work. Read-only: shows the git command, never runs it.

import SwiftUI

extension Worktree.Status {
    var label: String {
        switch self {
        case .prunable: return "Folder gone"
        case .locked: return "Locked"
        case .dirty: return "Uncommitted changes"
        case .active: return "Active, protected"
        case .done: return "Done"
        case .idle: return "Idle, unmerged"
        case .unknown: return "Unknown"
        }
    }
    /// Sort order for the Status column: finished first, protected last.
    var rank: Int {
        switch self {
        case .done: return 0
        case .prunable: return 1
        case .idle: return 2
        case .unknown: return 3
        case .active: return 4
        case .dirty: return 5
        case .locked: return 6
        }
    }
    var color: Color {
        switch self {
        case .done, .prunable: return .green
        case .idle: return .secondary
        case .dirty, .locked, .active: return .orange
        case .unknown: return .secondary
        }
    }
}

struct WorktreeRow: Identifiable {
    let id: String
    let wt: Worktree
    let display: String
    let repo: String
    let branch: String
    let size: UInt64
    let builds: UInt64
    let days: Int64
    let rank: Int

    init(_ w: Worktree, report: Report) {
        wt = w
        id = w.path
        display = report.tilde(w.path)
        repo = report.tilde(w.repo)
        branch = w.branch ?? w.head.map { "detached \($0)" } ?? "?"
        size = w.size ?? 0
        builds = w.artifactBytes
        days = w.lastActive.map { report.daysAgo($0) } ?? Int64.max
        rank = w.status.rank
    }
}

/// Shell-quote for copying a command into Terminal.
private func shellLine(_ argv: [String]) -> String {
    argv.map { a in
        a.allSatisfy({ $0.isLetter || $0.isNumber || "-_./~@:".contains($0) }) ? a
            : "'" + a.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }.joined(separator: " ")
}

struct WorktreesView: View {
    let report: Report
    @State private var search = ""
    @State private var sort = [KeyPathComparator(\WorktreeRow.size, order: .reverse)]
    @State private var selection = Set<WorktreeRow.ID>()

    var rows: [WorktreeRow] {
        report.worktrees.map { WorktreeRow($0, report: report) }
            .filter { search.isEmpty || $0.display.localizedCaseInsensitiveContains(search) || $0.branch.localizedCaseInsensitiveContains(search) }
            .sorted(using: sort)
    }

    var body: some View {
        let all = report.worktrees
        let total = all.reduce(0) { $0 + ($1.size ?? 0) }
        let builds = all.reduce(0) { $0 + $1.artifactBytes }
        let done = all.filter { $0.status == .done || $0.status == .prunable }
        let repos = Set(all.map(\.repo)).count
        VStack(alignment: .leading, spacing: 14) {
            ScreenHeader(
                title: "Worktrees",
                subtitle: all.isEmpty
                    ? "No linked git worktrees found."
                    : "\(all.count) worktrees of \(repos) repositor\(repos == 1 ? "y" : "ies"); \(Fmt.size(builds)) of it is build folders. \(done.count) look finished.",
                total: total)
            Table(rows, selection: $selection, sortOrder: $sort) {
                TableColumn("Worktree", value: \.display) { r in
                    HStack(spacing: 6) {
                        Text(r.display).lineLimit(1).truncationMode(.middle).help(r.wt.path)
                        if let c = r.wt.createdBy { Chip(text: c, color: .accentColor) }
                    }
                }
                .width(min: 200, ideal: 300)
                TableColumn("Size", value: \.size) { r in
                    if r.wt.size != nil { SizeText(bytes: r.size) } else { Text("–").foregroundColor(.secondary) }
                }
                .width(min: 64, ideal: 72)
                TableColumn("Build folders", value: \.builds) { r in SizeText(bytes: r.builds).foregroundColor(.secondary) }
                    .width(min: 70, ideal: 90)
                TableColumn("Last active", value: \.days) { r in
                    Text(r.days == Int64.max ? "–" : Fmt.age(days: r.days))
                }
                .width(min: 80, ideal: 95)
                TableColumn("Status", value: \.rank) { r in
                    Text(r.wt.status.label).foregroundColor(r.wt.status.color)
                }
                .width(min: 110, ideal: 140)
                TableColumn("Branch", value: \.branch) { r in
                    Text(r.branch).lineLimit(1).truncationMode(.middle).foregroundColor(.secondary).help(r.branch)
                }
                .width(min: 100, ideal: 200)
            }
            .contextMenu(forSelectionType: WorktreeRow.ID.self) { ids in
                if let id = ids.first, let w = all.first(where: { $0.path == id }) {
                    Button("Reveal in Finder") { Actions.reveal(w.exists ? w.path : w.repo) }
                    Button("Copy Path") { Actions.copy(w.path) }
                    if let cmd = w.suggested {
                        Button("Copy Command") { Actions.copy(shellLine(cmd)) }
                    }
                }
            } primaryAction: { ids in
                if let id = ids.first, let w = all.first(where: { $0.path == id }) {
                    Actions.reveal(w.exists ? w.path : w.repo)
                }
            }
            if selection.count == 1, let id = selection.first, let w = all.first(where: { $0.path == id }) {
                WorktreeDetail(report: report, wt: w)
            }
            Text("Nothing here is removed by SpaceSense yet. Active (14 days), locked worktrees and ones with uncommitted changes are never suggested. Removing a worktree keeps its branch, but also deletes git-ignored files in it such as .env.")
                .font(.caption).foregroundColor(.secondary)
        }
        .padding(24)
        .searchable(text: $search, prompt: "Filter worktrees")
    }
}

struct WorktreeDetail: View {
    let report: Report
    let wt: Worktree

    var reasons: [String] {
        var r: [String] = []
        let base = wt.base ?? "the base branch"
        switch wt.status {
        case .prunable: r.append("Its folder was deleted, but git still lists it.")
        case .locked: r.append("Locked with git worktree lock.")
        case .dirty: r.append("Has uncommitted or untracked changes: removing it would lose work.")
        case .active: r.append("Touched in the last 14 days.")
        case .unknown: r.append("Git state couldn't be read.")
        default: break
        }
        if wt.merged == true { r.append("Branch has no commits that aren't in \(base).") }
        if wt.merged == false { r.append("Branch has commits not in \(base); removing the worktree keeps the branch.") }
        if wt.upstreamGone { r.append("Its upstream branch was deleted (usually: PR merged).") }
        return r
    }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("Worktree of \(report.tilde(wt.repo))").font(.headline)
                    Spacer()
                    Button { Actions.reveal(wt.repo) } label: { Label("Main repository", systemImage: "arrow.right.circle") }
                        .buttonStyle(.borderless)
                }
                ForEach(reasons, id: \.self) { Text("• " + $0).font(.callout) }
                if let cmd = wt.suggested {
                    HStack {
                        Text(shellLine(cmd))
                            .font(.system(.callout, design: .monospaced))
                            .textSelection(.enabled)
                            .lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Button("Copy") { Actions.copy(shellLine(cmd)) }
                    }
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Color.secondary.opacity(0.08)))
                }
            }
        }
    }
}
