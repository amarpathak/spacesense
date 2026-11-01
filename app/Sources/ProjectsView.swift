// Projects with build artifacts, and large files. Sortable tables.

import SwiftUI

struct ProjectRow: Identifiable {
    let id: String
    let project: Project
    let display: String
    let size: UInt64
    let days: Int64
    let kinds: String
    let artifacts: String
    let git: String
    let status: String
    let statusRank: Int

    init(_ p: Project, report: Report) {
        project = p
        id = p.path
        display = report.tilde(p.path)
        size = p.artifactBytes
        days = report.daysAgo(p.lastActive)
        kinds = p.kinds.joined(separator: ", ")
        artifacts = p.artifacts.map { $0.name }.joined(separator: " ")
        var g = "no git"
        if let gi = p.git {
            g = gi.branch ?? "?"
            if gi.dirty == true { g += " · changes" }
            if let w = gi.worktreeOf { g += " · worktree of \(report.tilde(w))" }
        }
        git = g
        if days < 14 {
            status = "Active, protected"
            statusRank = 0
        } else if days < 30 {
            status = "Recent"
            statusRank = 1
        } else {
            status = "Stale, preselected"
            statusRank = 2
        }
    }
}

struct ProjectsView: View {
    let report: Report
    enum Filter: String, CaseIterable, Identifiable {
        case all = "All", stale = "Untouched 30+ days", active = "Active"
        var id: String { rawValue }
    }

    @State private var filter: Filter = .all
    @State private var search = ""
    @State private var sort = [KeyPathComparator(\ProjectRow.size, order: .reverse)]
    @State private var selection = Set<ProjectRow.ID>()
    @EnvironmentObject var cleaner: Cleaner

    var rows: [ProjectRow] {
        report.projects.map { ProjectRow($0, report: report) }
            .filter { r in
                switch filter {
                case .all: return true
                case .stale: return r.days >= 30
                case .active: return r.days < 14
                }
            }
            .filter { search.isEmpty || $0.display.localizedCaseInsensitiveContains(search) }
            .sorted(using: sort)
    }

    var body: some View {
        let all = report.projects
        let stale = all.filter { report.daysAgo($0.lastActive) >= 30 }
        let rows = self.rows
        VStack(alignment: .leading, spacing: 14) {
            ScreenHeader(
                title: "Projects",
                subtitle: "\(all.count) projects with build folders. \(stale.count) untouched for 30+ days hold \(Fmt.size(stale.reduce(0) { $0 + $1.artifactBytes })).",
                total: all.reduce(0) { $0 + $1.artifactBytes })
            HStack {
                Picker("", selection: $filter) {
                    ForEach(Filter.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .frame(width: 380)
                Spacer()
                Text("\(rows.count) shown · \(Fmt.size(rows.reduce(0) { $0 + $1.size }))")
                    .foregroundColor(.secondary).font(.callout.monospacedDigit())
            }
            Table(rows, selection: $selection, sortOrder: $sort) {
                TableColumn("Project", value: \.display) { r in
                    Text(r.display).lineLimit(1).truncationMode(.middle).help(r.project.path)
                }
                .width(min: 180, ideal: 260)
                TableColumn("Size", value: \.size) { r in SizeText(bytes: r.size) }
                    .width(min: 64, ideal: 72)
                TableColumn("Last active", value: \.days) { r in Text(Fmt.age(days: r.days)) }
                    .width(min: 80, ideal: 95)
                TableColumn("Status", value: \.statusRank) { r in
                    Text(r.status).foregroundColor(r.statusRank == 0 ? .orange : (r.statusRank == 2 ? .green : .secondary))
                }
                .width(min: 100, ideal: 120)
                TableColumn("Build folders", value: \.artifacts) { r in Text(r.artifacts).lineLimit(1) }
                    .width(min: 100, ideal: 140)
                TableColumn("Git", value: \.git) { r in Text(r.git).lineLimit(1).truncationMode(.tail).foregroundColor(.secondary).help(r.git) }
                    .width(min: 100, ideal: 200)
            }
            .contextMenu(forSelectionType: ProjectRow.ID.self) { ids in
                if let id = ids.first {
                    Button("Reveal in Finder") { Actions.reveal(id) }
                    Button("Copy Path") { Actions.copy(id) }
                }
            } primaryAction: { ids in
                if let id = ids.first { Actions.reveal(id) }
            }
            if selection.count == 1, let id = selection.first, let p = report.projects.first(where: { $0.path == id }) {
                ProjectDetail(report: report, project: p)
            }
            HStack {
                let cleanable = selection.filter { cleaner.isCandidate($0) }
                Text(selection.isEmpty ? "Select projects (⌘-click for several) to clean their build folders." :
                        "\(selection.count) selected\(cleanable.count < selection.count ? ", \(selection.count - cleanable.count) active or protected" : "")")
                    .font(.callout).foregroundColor(.secondary)
                Spacer()
                Button("Clean build folders of \(cleanable.count) project\(cleanable.count == 1 ? "" : "s")…") {
                    cleaner.open(with: Array(cleanable))
                }
                .disabled(cleanable.isEmpty)
            }
            Text("Active in the last 14 days: never preselected. Build folders that git doesn't ignore are never cleaned. Uncommitted changes aren't affected by removing git-ignored folders.")
                .font(.caption).foregroundColor(.secondary)
        }
        .padding(24)
        .searchable(text: $search, prompt: "Filter projects")
    }
}

struct ProjectDetail: View {
    let report: Report
    let project: Project
    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(project.artifacts, id: \.path) { a in
                    HStack(spacing: 10) {
                        Text(a.name).font(.system(.body, design: .monospaced)).frame(width: 130, alignment: .leading)
                        SafetyBadge(safety: a.safety)
                        if a.gitIgnored == false {
                            Chip(text: "not git-ignored: never cleaned", color: .orange, symbol: "exclamationmark.triangle")
                        }
                        Text("rebuilt by \(a.regenerate)").foregroundColor(.secondary).lineLimit(1)
                        Spacer()
                        SizeText(bytes: a.size)
                        Button { Actions.reveal(a.path) } label: { Image(systemName: "arrow.right.circle") }
                            .buttonStyle(.borderless)
                    }
                    .font(.callout)
                }
            }
        }
    }
}

struct LargeFilesView: View {
    let report: Report

    struct Row: Identifiable {
        let id: String
        let name: String
        let folder: String
        let size: UInt64
        let days: Int64
        let kind: String
    }

    @State private var sort = [KeyPathComparator(\Row.size, order: .reverse)]
    @State private var selection = Set<Row.ID>()
    @EnvironmentObject var cleaner: Cleaner

    var body: some View {
        let rows = report.largeFiles.map { f in
            Row(id: f.path, name: (f.path as NSString).lastPathComponent,
                folder: report.tilde((f.path as NSString).deletingLastPathComponent),
                size: f.size, days: report.daysAgo(f.mtime), kind: f.kind)
        }.sorted(using: sort)
        VStack(alignment: .leading, spacing: 14) {
            ScreenHeader(
                title: "Large Files",
                subtitle: "Files of 500 MB or more outside caches and build folders. Yours to judge: SpaceSense only ever moves these to the Trash.",
                total: rows.reduce(0) { $0 + $1.size })
            Table(rows, selection: $selection, sortOrder: $sort) {
                TableColumn("Name", value: \.name) { r in Text(r.name).lineLimit(1).truncationMode(.middle).help(r.id) }
                    .width(min: 220, ideal: 340)
                TableColumn("Size", value: \.size) { r in SizeText(bytes: r.size) }
                    .width(min: 70, ideal: 80)
                TableColumn("Modified", value: \.days) { r in Text(Fmt.age(days: r.days)) }
                    .width(min: 90, ideal: 110)
                TableColumn("Kind", value: \.kind) { r in Text(r.kind) }
                    .width(min: 80, ideal: 100)
                TableColumn("Folder", value: \.folder) { r in
                    Text(r.folder).lineLimit(1).truncationMode(.middle).foregroundColor(.secondary)
                }
                .width(min: 150, ideal: 300)
            }
            .contextMenu(forSelectionType: Row.ID.self) { ids in
                if let id = ids.first {
                    Button("Reveal in Finder") { Actions.reveal(id) }
                    Button("Copy Path") { Actions.copy(id) }
                }
            } primaryAction: { ids in
                if let id = ids.first { Actions.reveal(id) }
            }
            HStack {
                Text("Double-click a file to reveal it in Finder. ⌘-click to select several.").font(.caption).foregroundColor(.secondary)
                Spacer()
                let picked = selection.filter { cleaner.isCandidate($0) }
                Button("Move \(picked.count) file\(picked.count == 1 ? "" : "s") to Trash…") {
                    cleaner.open(with: Array(picked))
                }
                .disabled(picked.isEmpty)
            }
        }
        .padding(24)
    }
}
