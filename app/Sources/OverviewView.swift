// Overview: disk bar, reclaimable total, categories, where the home folder goes.

import SwiftUI

struct OverviewView: View {
    @EnvironmentObject var model: AppModel
    let report: Report
    let plan: Plan

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                DiskCard(report: report)
                ReclaimCard(plan: plan, onPreview: { model.cleaner.open() })
                if !report.needsFullDiskAccess.isEmpty {
                    FullDiskAccessBanner(count: report.needsFullDiskAccess.count)
                }
                Text("What's using space").font(.title2.weight(.semibold))
                CategoryGrid(report: report)
                HomeBreakdown(report: report)
                SystemDataCard(report: report)
            }
            .padding(24)
            .frame(maxWidth: 1000, alignment: .leading)
        }
    }
}

struct DiskCard: View {
    let report: Report

    struct Segment: Identifiable {
        let id: String
        let bytes: UInt64
        let color: Color
        let help: String
    }

    var body: some View {
        Card {
            if let v = report.volume {
                let volumeUsed = v.volumeUsedBytes ?? v.usedBytes
                let home = min(report.scan.totalBytes, volumeUsed)
                let segments = [
                    Segment(id: "Your home folder", bytes: home, color: .accentColor,
                            help: "Everything SpaceSense scanned in \(report.tilde(report.root))."),
                    Segment(id: "Apps & other data", bytes: volumeUsed - home, color: .purple,
                            help: "Applications, other users and system-wide caches on the Data volume (outside your home folder)."),
                    Segment(id: "macOS", bytes: v.usedBytes - min(v.usedBytes, volumeUsed), color: .gray,
                            help: "macOS itself, its swap and update files: separate volumes in the same APFS container."),
                    Segment(id: "Free", bytes: v.freeBytes, color: Color.secondary.opacity(0.2), help: "Free space."),
                ]
                let pct = Int(Double(v.usedBytes) / Double(max(1, v.totalBytes)) * 100)
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .firstTextBaseline) {
                        Text("Macintosh HD").font(.title2.weight(.semibold))
                        Spacer()
                        Text("\(Fmt.size(v.freeBytes)) free of \(Fmt.size(v.totalBytes))")
                            .font(.title3.monospacedDigit())
                            .foregroundColor(pct >= 90 ? .red : .primary)
                    }
                    GeometryReader { g in
                        HStack(spacing: 2) {
                            ForEach(segments) { s in
                                Rectangle().fill(s.color)
                                    .frame(width: max(2, g.size.width * CGFloat(Double(s.bytes) / Double(max(1, v.totalBytes)))))
                                    .help("\(s.id): \(Fmt.size(s.bytes)). \(s.help)")
                            }
                        }
                    }
                    .frame(height: 22)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                    HStack(spacing: 18) {
                        ForEach(segments) { s in
                            HStack(spacing: 6) {
                                Circle().fill(s.color).frame(width: 9, height: 9)
                                Text(s.id).font(.callout)
                                Text(Fmt.size(s.bytes)).font(.callout.monospacedDigit()).foregroundColor(.secondary)
                            }
                            .help(s.help)
                        }
                    }
                    if pct >= 90 {
                        Text("Your disk is \(pct)% full. macOS gets slow and unstable with less than about 10 GB free.")
                            .font(.callout).foregroundColor(.red)
                    }
                    if let cap = VolumeCapacity.read(report.root), cap.purgeable > 1_000_000_000 {
                        Text("macOS can also free about \(Fmt.size(UInt64(cap.purgeable))) of purgeable space on its own (iCloud files, snapshots, caches it manages) when it needs room.")
                            .font(.callout).foregroundColor(.secondary)
                    }
                }
            }
        }
    }
}

struct ReclaimCard: View {
    let plan: Plan
    let onPreview: () -> Void
    var body: some View {
        Card {
            HStack(alignment: .center, spacing: 20) {
                Image(systemName: "sparkles").font(.system(size: 34)).foregroundColor(.green)
                VStack(alignment: .leading, spacing: 4) {
                    Text("≈ \(Fmt.size(plan.totalBytes)) could be freed safely")
                        .font(.system(size: 26, weight: .semibold))
                    Text("Caches, rebuildable data and build folders of projects untouched for 30+ days. You choose what to clean and confirm every step; nothing is removed on its own.")
                        .foregroundColor(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer()
                Button("Choose what to clean…", action: onPreview)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
            }
        }
    }
}

struct FullDiskAccessBanner: View {
    let count: Int
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "lock.shield").foregroundColor(.orange).font(.title2)
            VStack(alignment: .leading, spacing: 2) {
                Text("\(count) locations need Full Disk Access to measure").font(.headline)
                Text("Mail, Messages, Safari, iPhone backups and the Trash are protected by macOS. SpaceSense works without access; their sizes just aren't counted.")
                    .font(.callout).foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text("To allow it: in the Full Disk Access list click +, choose SpaceSense (shown in Finder), or drag it into the list. Then quit and reopen SpaceSense.")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            Button("Open Settings…") { Actions.openFullDiskAccessSettings() }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.orange.opacity(0.08)))
    }
}

struct CategoryGrid: View {
    @EnvironmentObject var model: AppModel
    let report: Report

    struct Item: Identifiable {
        let id: String
        let symbol: String
        let bytes: UInt64
        let detail: String
        let screen: Screen
    }

    var items: [Item] {
        let dev = report.rules(for: .developer)
        let docker = dev.filter { $0.category == "docker" }.reduce(0) { $0 + $1.size }
        let pkg = dev.filter { $0.category != "docker" }.reduce(0) { $0 + $1.size }
        let sys = report.rules(for: .system).reduce(0) { $0 + $1.size }
        let proj = report.projects.reduce(0) { $0 + $1.artifactBytes }
        let nm = report.projects.flatMap { $0.artifacts }.filter { $0.name == "node_modules" }.count
        let large = report.largeFiles.reduce(0) { $0 + $1.size }
        var v = [
            Item(id: "Project build folders", symbol: "folder.badge.gearshape", bytes: proj,
                 detail: "\(report.projects.count) projects, \(nm) node_modules. Rebuilt by install / build.", screen: .projects),
            Item(id: "Developer caches & tools", symbol: "shippingbox", bytes: pkg,
                 detail: "npm, Yarn, Homebrew, pip, Cargo, toolchains… Re-downloaded when needed.", screen: .developer),
            Item(id: "App caches & data", symbol: "app.badge", bytes: sys,
                 detail: "Per-app caches, the Claude VM, logs, Trash.", screen: .system),
            Item(id: "Large files", symbol: "doc.zipper", bytes: large,
                 detail: "\(report.largeFiles.count) files over 500 MB: installers, archives, videos.", screen: .large),
        ]
        if docker > 0 {
            let leftover = dev.contains { $0.category == "docker" && $0.leftover }
            v.insert(Item(id: "Docker", symbol: "shippingbox.circle", bytes: docker,
                          detail: leftover ? "Docker Desktop is uninstalled, but its disk image is still here." : "Images, containers, volumes and build cache.",
                          screen: .developer), at: 2)
        }
        return v.sorted { $0.bytes > $1.bytes }
    }

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 280), spacing: 14)], spacing: 14) {
            ForEach(items) { it in
                Button {
                    model.screen = it.screen
                } label: {
                    Card {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Image(systemName: it.symbol).font(.title2).foregroundColor(.accentColor)
                                Spacer()
                                SizeText(bytes: it.bytes, font: .title2.weight(.semibold))
                            }
                            Text(it.id).font(.headline)
                            Text(it.detail).font(.callout).foregroundColor(.secondary)
                                .lineLimit(2, reservesSpace: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
                .buttonStyle(.plain)
            }
        }
    }
}

struct HomeBreakdown: View {
    @EnvironmentObject var model: AppModel
    let report: Report
    var body: some View {
        let top = Array(report.topDirs.prefix(12))
        let maxSize = Double(top.first?.size ?? 1)
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text("Where your home folder goes").font(.title2.weight(.semibold))
                Spacer()
                Button("Browse all folders") { model.browse(report.root) }.buttonStyle(.link)
                Text("\(Fmt.size(report.scan.totalBytes)) in \(report.tilde(report.root))").foregroundColor(.secondary)
            }
            Card {
                VStack(spacing: 8) {
                    ForEach(top, id: \.path) { d in
                        Button {
                            model.browse(d.path)
                        } label: {
                            HStack(spacing: 12) {
                                Text((d.path as NSString).lastPathComponent)
                                    .frame(width: 180, alignment: .leading)
                                    .lineLimit(1)
                                Meter(fraction: Double(d.size) / maxSize)
                                SizeText(bytes: d.size).frame(width: 80, alignment: .trailing)
                                Image(systemName: "chevron.right").foregroundColor(.secondary)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help("Show what's inside, with what's safe to delete")
                    }
                }
            }
            Text("Sizes count real disk use (like du), hard links once. APFS clones may be counted twice, so totals can overstate a little.")
                .font(.caption).foregroundColor(.secondary)
        }
    }
}

struct SystemDataCard: View {
    @EnvironmentObject var model: AppModel
    let report: Report
    @State private var expanded: Set<String> = []

    var body: some View {
        let parts = report.systemData
        let total = parts.reduce(0) { $0 + ($1.size ?? 0) }
        let maxSize = Double(parts.compactMap(\.size).max() ?? 1)
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text("What's in “System Data”").font(.title2.weight(.semibold))
                Spacer()
                Text("≈ \(Fmt.size(total)) measured").foregroundColor(.secondary)
            }
            Text("System Settings → Storage shows one grey “System Data” bar you can't open. These are the pieces it's made of on this Mac. Apple doesn't publish its formula, so its number won't match exactly.")
                .font(.callout).foregroundColor(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Card {
                VStack(spacing: 4) {
                    ForEach(parts) { p in
                        row(p, maxSize: maxSize)
                        if p.id != parts.last?.id { Divider() }
                    }
                }
            }
        }
    }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }

    /// Inside the scanned folder the folder browser can show it; otherwise Finder.
    private func open(_ path: String) {
        if path == report.root || path.hasPrefix(report.root + "/") { model.browse(path) } else { Actions.reveal(path) }
    }

    @ViewBuilder private func row(_ p: SystemPart, maxSize: Double) -> some View {
        let isOpen = expanded.contains(p.id)
        VStack(alignment: .leading, spacing: 8) {
            Button { toggle(p.id) } label: {
                HStack(spacing: 12) {
                    Image(systemName: "chevron.right")
                        .rotationEffect(.degrees(isOpen ? 90 : 0))
                        .foregroundColor(.secondary).frame(width: 12)
                    Text(p.name).frame(width: 260, alignment: .leading).lineLimit(1)
                    if p.managedByMacos {
                        Chip(text: "managed by macOS", color: .secondary, symbol: "gearshape")
                    }
                    Meter(fraction: Double(p.size ?? 0) / maxSize, color: p.managedByMacos ? .gray : .accentColor)
                    Group {
                        if let s = p.size { SizeText(bytes: s) } else { Text("–").foregroundColor(.secondary) }
                    }
                    .frame(width: 80, alignment: .trailing)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if isOpen {
                VStack(alignment: .leading, spacing: 6) {
                    Text(p.explanation).font(.callout).foregroundColor(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let n = p.note { Text(n).font(.callout) }
                    ForEach(p.top ?? [], id: \.path) { t in
                        Button { open(t.path) } label: {
                            HStack {
                                Text(report.tilde(t.path)).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                SizeText(bytes: t.size, font: .callout).foregroundColor(.secondary)
                                Image(systemName: "chevron.right").font(.caption).foregroundColor(.secondary)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .font(.callout)
                    }
                    HStack(spacing: 14) {
                        if let s = p.screen.flatMap(screen(for:)) {
                            Button("See what's cleanable in \(s.rawValue)") { model.screen = s }.buttonStyle(.link)
                        }
                        if p.paths.count == 1, let path = p.paths.first {
                            Button(p.screen == "folders" ? "Browse folder" : "Show in Finder") {
                                p.screen == "folders" ? open(path) : Actions.reveal(path)
                            }
                            .buttonStyle(.link)
                        }
                    }
                    .font(.callout)
                }
                .padding(.leading, 24)
                .padding(.bottom, 6)
            }
        }
        .padding(.vertical, 4)
    }

    private func screen(for id: String) -> Screen? {
        switch id {
        case "developer": return .developer
        case "system": return .system
        default: return nil
        }
    }
}
