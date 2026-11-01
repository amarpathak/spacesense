// Folders screen: drill into any folder of the last scan and see, per subfolder,
// its size, the app it belongs to and whether it's safe to delete.

import AppKit
import SwiftUI

struct Explanation: Decodable, Hashable {
    let label: String
    let safety: Safety?
    let explanation: String
    let ruleId: String?
    let ownerHint: String?
}

struct FolderItem: Decodable, Identifiable, Hashable {
    var id: String { path }
    let name: String
    let path: String
    let size: UInt64
    let files: UInt64
    let newestMtime: Int64
    let hasChildren: Bool
    let error: String?
    let info: Explanation
}

struct FolderListing: Decodable {
    let path: String
    let size: UInt64
    let files: UInt64
    let info: Explanation
    let filesBytes: UInt64
    let directFiles: UInt64
    let items: [FolderItem]
    let moreItems: Int
    let moreBytes: UInt64
    let error: String?
}

enum ListingResult {
    case success(FolderListing)
    case failure(String)
}

enum FolderSource {
    static func listing(_ path: String) -> ListingResult {
        let raw = path.withCString { ss_folder($0, 300) }
        defer { ss_free(raw) }
        guard let raw = raw else { return .failure("no result") }
        let data = Data(String(cString: raw).utf8)
        struct Err: Decodable { let error: String }
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        if let e = try? d.decode(Err.self, from: data) { return .failure(e.error) }
        do { return .success(try d.decode(FolderListing.self, from: data)) } catch { return .failure("\(error)") }
    }
}

/// Which app a folder belongs to, from its name (bundle id or app name).
struct AppOwner {
    enum Status { case installed, macOS, notFound }
    let name: String
    let icon: NSImage?
    let status: Status

    private static var cache: [String: AppOwner?] = [:]

    static func resolve(_ hint: String?) -> AppOwner? {
        guard let hint = hint, !hint.isEmpty else { return nil }
        if let c = cache[hint] { return c }
        let r = lookup(hint)
        cache[hint] = r
        return r
    }

    private static func lookup(_ hint: String) -> AppOwner? {
        let ws = NSWorkspace.shared
        // Group containers: "TEAMID.group.com.x" or "group.com.x".
        var id = hint
        if let r = id.range(of: "group.") { id = String(id[r.upperBound...]) }
        let looksLikeBundleId = id.contains(".") && !id.contains(" ") && id.split(separator: ".").count >= 2
        if looksLikeBundleId {
            if let url = ws.urlForApplication(withBundleIdentifier: id) {
                return AppOwner(name: FileManager.default.displayName(atPath: url.path).replacingOccurrences(of: ".app", with: ""),
                                icon: ws.icon(forFile: url.path), status: .installed)
            }
            if id.hasPrefix("com.apple.") {
                return AppOwner(name: "macOS / Apple", icon: nil, status: .macOS)
            }
            return AppOwner(name: id, icon: nil, status: .notFound)
        }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        // Vendor folder names that don't match the app's name.
        let aliases = [
            "Google": "Google Chrome", "Chrome": "Google Chrome", "Code": "Visual Studio Code",
            "BraveSoftware": "Brave Browser", "Microsoft": "Microsoft Teams", "Mozilla": "Firefox",
            "Firefox": "Firefox", "Spotify": "Spotify", "zoom.us": "zoom.us", "JetBrains": "JetBrains Toolbox",
        ]
        let names = [aliases[hint], hint].compactMap { $0 }
        for name in names {
            for dir in ["/Applications", "/System/Applications", home + "/Applications", "/Applications/Utilities"] {
                let p = dir + "/" + name + ".app"
                if FileManager.default.fileExists(atPath: p) {
                    return AppOwner(name: name, icon: ws.icon(forFile: p), status: .installed)
                }
            }
        }
        return nil // unknown: make no claim
    }
}

struct FolderBrowser: View {
    @EnvironmentObject var model: AppModel
    let report: Report

    var path: String { model.folderPath ?? report.root }

    var body: some View {
        let result = FolderSource.listing(path)
        VStack(alignment: .leading, spacing: 0) {
            Breadcrumbs(report: report, path: path)
                .padding(.horizontal, 24)
                .padding(.vertical, 10)
            Divider()
            switch result {
            case .failure(let e):
                VStack(spacing: 8) {
                    Text("Can't show this folder").font(.title3)
                    Text(e).foregroundColor(.secondary)
                    Text("Folder browsing needs a scan in this session (⌘R).").font(.callout).foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .success(let l):
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        FolderHeader(report: report, listing: l)
                        SafetyLegend()
                        if let common = commonExplanation(l.items) {
                            Text(common).font(.callout).foregroundColor(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        VStack(spacing: 0) {
                            ForEach(l.items) { item in
                                FolderRow(report: report, item: item, parentSize: l.size,
                                          showExplanation: item.info.explanation != commonExplanation(l.items)) {
                                    if item.hasChildren { model.folderPath = item.path }
                                }
                                Divider()
                            }
                            if l.moreItems > 0 {
                                InfoRow(text: "\(l.moreItems) smaller folders", bytes: l.moreBytes)
                                Divider()
                            }
                            if l.filesBytes > 0 {
                                InfoRow(text: "Files directly in this folder (\(Fmt.count(l.directFiles)))", bytes: l.filesBytes)
                            }
                        }
                        .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .controlBackgroundColor)))
                        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.secondary.opacity(0.15)))
                    }
                    .padding(24)
                    .frame(maxWidth: 1100, alignment: .leading)
                }
            }
        }
    }
}

/// The explanation most rows share (e.g. every app folder in Application
/// Support): shown once above the list instead of on every row.
func commonExplanation(_ items: [FolderItem]) -> String? {
    var counts: [String: Int] = [:]
    for i in items where !i.info.explanation.isEmpty { counts[i.info.explanation, default: 0] += 1 }
    guard let (text, n) = counts.max(by: { $0.value < $1.value }), n >= 3 else { return nil }
    return text
}

struct Breadcrumbs: View {
    @EnvironmentObject var model: AppModel
    let report: Report
    let path: String

    var body: some View {
        let root = report.root
        let rel = path.hasPrefix(root) ? String(path.dropFirst(root.count)) : ""
        let parts = rel.split(separator: "/").map(String.init)
        HStack(spacing: 4) {
            Button {
                if parts.count > 0 { model.folderPath = parts.count == 1 ? root : root + "/" + parts.dropLast().joined(separator: "/") }
            } label: { Image(systemName: "chevron.left") }
                .disabled(parts.isEmpty)
                .help("Up one folder")
            Button(report.tilde(root)) { model.folderPath = root }.buttonStyle(.link)
            ForEach(Array(parts.enumerated()), id: \.offset) { i, name in
                Image(systemName: "chevron.right").font(.caption).foregroundColor(.secondary)
                if i == parts.count - 1 {
                    Text(name).fontWeight(.semibold)
                } else {
                    Button(name) { model.folderPath = root + "/" + parts[0...i].joined(separator: "/") }.buttonStyle(.link)
                }
            }
            Spacer()
            Button("Reveal in Finder") { Actions.reveal(path) }
        }
    }
}

struct FolderHeader: View {
    @EnvironmentObject var cleaner: Cleaner
    let report: Report
    let listing: FolderListing
    var body: some View {
        let owner = AppOwner.resolve(listing.info.ownerHint)
        Card {
            HStack(alignment: .top, spacing: 14) {
                AppIcon(owner: owner, size: 44)
                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text((listing.path as NSString).lastPathComponent).font(.title2.weight(.semibold))
                        if let s = listing.info.safety { SafetyBadge(safety: s) }
                        Spacer()
                        SizeText(bytes: listing.size, font: .title2.weight(.semibold))
                        if cleanable(listing.path, info: listing.info, report: report) {
                            Button(listing.info.safety == .review ? "Move to Trash…" : "Clean this folder…") {
                                let t = listing.info.ruleId.flatMap { listing.info.label.hasPrefix("Part of") ? nil : $0 } ?? ("folder:" + listing.path)
                                cleaner.open(with: [t])
                            }
                        }
                    }
                    Text(ownerLine(owner, label: listing.info.label)).foregroundColor(.secondary)
                    if !listing.info.explanation.isEmpty {
                        Text(listing.info.explanation).fixedSize(horizontal: false, vertical: true)
                    }
                    Text("\(Fmt.count(listing.files)) files").font(.callout).foregroundColor(.secondary)
                    if let e = listing.error {
                        Label("Couldn't read: \(e)", systemImage: "lock").font(.callout).foregroundColor(.orange)
                    }
                }
            }
        }
    }
}

func ownerLine(_ owner: AppOwner?, label: String) -> String {
    guard let o = owner else { return label }
    switch o.status {
    case .installed: return "\(label) · \(o.name)"
    case .macOS: return "\(label) · macOS"
    case .notFound: return "\(label) · \(o.name): no app with this ID is installed (may be leftover)"
    }
}

struct AppIcon: View {
    let owner: AppOwner?
    var size: CGFloat = 28
    var body: some View {
        if let icon = owner?.icon {
            Image(nsImage: icon).resizable().frame(width: size, height: size)
        } else {
            Image(systemName: owner?.status == .notFound ? "questionmark.folder" : "folder.fill")
                .resizable().scaledToFit()
                .foregroundColor(owner?.status == .notFound ? .orange : .accentColor.opacity(0.8))
                .frame(width: size * 0.8, height: size * 0.8)
                .frame(width: size, height: size)
        }
    }
}

/// Folders that can be offered for cleaning: known verdict (not "don't
/// delete"), and not a container of many apps' or the user's data. The core
/// re-checks all of this.
func cleanable(_ path: String, info: Explanation, report: Report) -> Bool {
    guard let s = info.safety, s != .neverAuto else { return false }
    let home = report.home
    guard path.hasPrefix(home + "/") else { return false }
    let parts = path.dropFirst(home.count + 1).split(separator: "/")
    if parts.count == 1 && ["Library", "Desktop", "Documents", "Downloads", "Pictures", "Movies", "Music", "Applications", "Public"].contains(String(parts[0])) { return false }
    if parts.count == 2 && parts[0] == "Library" { return false }
    return true
}

struct FolderRow: View {
    @EnvironmentObject var cleaner: Cleaner
    let report: Report
    let item: FolderItem
    let parentSize: UInt64
    var showExplanation = true
    let open: () -> Void
    @State private var hover = false

    var body: some View {
        let owner = AppOwner.resolve(item.info.ownerHint)
        Button(action: open) {
            HStack(spacing: 12) {
                AppIcon(owner: owner)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Text(item.name).font(.body.weight(.medium)).lineLimit(1).truncationMode(.middle)
                        if let s = item.info.safety { SafetyBadge(safety: s) }
                        if owner?.status == .notFound {
                            Chip(text: "app not installed", color: .orange, symbol: "exclamationmark.circle")
                        }
                        if let e = item.error {
                            if e.hasPrefix("not scanned") {
                                Chip(text: "not scanned: cloud storage", color: .blue, symbol: "icloud")
                            } else if e.contains("iCloud") {
                                Chip(text: "in iCloud only", color: .blue, symbol: "icloud")
                            } else {
                                Chip(text: e.contains("not permitted") ? "needs Full Disk Access" : "couldn't read",
                                     color: .secondary, symbol: "lock")
                            }
                        }
                    }
                    Text(ownerLine(owner, label: item.info.label) + (item.info.explanation.isEmpty || !showExplanation ? "" : " — " + item.info.explanation))
                        .font(.callout).foregroundColor(.secondary)
                        .lineLimit(2)
                        .help(item.info.explanation)
                }
                Spacer(minLength: 16)
                VStack(alignment: .trailing, spacing: 4) {
                    SizeText(bytes: item.size, font: .body.weight(.semibold))
                    Meter(fraction: Double(item.size) / Double(max(1, parentSize)), color: (item.info.safety ?? .review).color.opacity(0.8), height: 4)
                        .frame(width: 110)
                    Text("modified \(Fmt.age(days: report.daysAgo(item.newestMtime)))").font(.caption).foregroundColor(.secondary)
                }
                .frame(width: 140, alignment: .trailing)
                if cleanable(item.path, info: item.info, report: report) && item.error == nil {
                    Button(item.info.safety == .review ? "Trash…" : "Clean…") {
                        cleaner.open(with: [target(item)])
                    }
                    .help(item.info.safety == .review ? "Move to the Trash, after you review it" : "Delete, after you review it")
                    .opacity(hover ? 1 : 0.55)
                }
                Image(systemName: "chevron.right").foregroundColor(item.hasChildren ? .secondary : .clear)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .contentShape(Rectangle())
            .background(hover ? Color.secondary.opacity(0.07) : Color.clear)
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .contextMenu {
            if cleanable(item.path, info: item.info, report: report) {
                Button(item.info.safety == .review ? "Move to Trash…" : "Clean…") { cleaner.open(with: [target(item)]) }
                Divider()
            }
            Button("Reveal in Finder") { Actions.reveal(item.path) }
            Button("Copy Path") { Actions.copy(item.path) }
        }
    }
}

/// A rule's own target when the folder is exactly a catalog location, else the folder.
func target(_ item: FolderItem) -> String {
    if let r = item.info.ruleId, !(item.info.label.hasPrefix("Part of")) { return r }
    return "folder:" + item.path
}

struct InfoRow: View {
    let text: String
    let bytes: UInt64
    var body: some View {
        HStack {
            Image(systemName: "doc.on.doc").foregroundColor(.secondary).frame(width: 28)
            Text(text).foregroundColor(.secondary)
            Spacer()
            SizeText(bytes: bytes).foregroundColor(.secondary)
            Image(systemName: "chevron.right").foregroundColor(.clear)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
    }
}

struct SafetyLegend: View {
    var body: some View {
        HStack(spacing: 16) {
            ForEach([Safety.cache, .regenerable, .review, .neverAuto], id: \.self) { s in
                HStack(spacing: 6) {
                    SafetyBadge(safety: s)
                    Text(s.short).font(.caption).foregroundColor(.secondary)
                }
            }
        }
    }
}

extension Safety {
    var short: String {
        switch self {
        case .cache: return "safe to delete"
        case .regenerable: return "rebuilt automatically"
        case .review: return "your/app data: look first"
        case .neverAuto: return "don't delete"
        }
    }
}
