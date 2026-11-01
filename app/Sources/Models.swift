// Codable mirror of the Rust report (core/src/report.rs, core/src/plan.rs).
// Decoded with .convertFromSnakeCase. Only fields the UI uses are declared.

import Foundation

struct AppPayload: Decodable {
    let report: Report
    let plan: Plan
}

struct Report: Decodable {
    let root: String
    let home: String
    let generatedAt: Int64
    let scan: ScanSummary
    let volume: Volume?
    let rules: [RuleReport]
    let projects: [Project]
    let largeFiles: [LargeFile]
    let needsFullDiskAccess: [String]
    let topDirs: [PathSize]
    let rootFilesBytes: UInt64
}

struct ScanSummary: Decodable {
    let totalBytes: UInt64
    let files: UInt64
    let dirs: UInt64
    let unreadableDirs: UInt64
    let hardlinkBytesDeduped: UInt64
    let elapsedMs: UInt64
}

struct Volume: Decodable {
    let totalBytes: UInt64
    let freeBytes: UInt64
    let usedBytes: UInt64
    let volumeUsedBytes: UInt64?
}

struct PathSize: Decodable, Hashable {
    let path: String
    let size: UInt64
    let error: String?
}

enum Safety: String, Decodable {
    case cache, regenerable, review
    case neverAuto = "never_auto"
}

struct Rule: Decodable {
    let explanation: String
    let comesBack: String
    let officialCmd: [String]?
    let dryRunCmd: [String]?
    let fallback: String
    let leftoverFallback: String?
    let leftoverExplanation: String?
    let docsUrl: String
    let tool: String?
    let generic: Bool
    let quitApps: [String]
    let special: String?
    let verified: String
}

struct RuleReport: Decodable, Identifiable {
    let id: String
    let name: String
    let category: String
    let safety: Safety
    let size: UInt64
    let found: Bool
    let paths: [PathSize]
    let details: [PathSize]?
    let items: [PathSize]?
    let tool: String?
    let toolMissing: Bool
    let leftover: Bool
    let runningApps: [String]?
    let note: String?
    let rule: Rule
}

struct Artifact: Decodable, Hashable {
    let path: String
    let name: String
    let size: UInt64
    let safety: Safety
    let regenerate: String
    let gitIgnored: Bool?
}

struct GitInfo: Decodable, Hashable {
    let root: String
    let branch: String?
    let dirty: Bool?
    let worktreeOf: String?
}

struct Project: Decodable, Identifiable, Hashable {
    var id: String { path }
    let path: String
    let kinds: [String]
    let artifacts: [Artifact]
    let artifactBytes: UInt64
    let lastActive: Int64
    let git: GitInfo?
    let group: String

    static func == (a: Project, b: Project) -> Bool { a.path == b.path }
    func hash(into h: inout Hasher) { h.combine(path) }
}

struct LargeFile: Decodable, Identifiable, Hashable {
    var id: String { path }
    let path: String
    let size: UInt64
    let mtime: Int64
    let kind: String
}

struct Plan: Decodable {
    var actions: [PlanAction]
    let skipped: [Skipped]
    let totalBytes: UInt64
}

struct PlanAction: Decodable, Identifiable {
    var id: String { target }
    /// Pass back to ss_plan / ss_clean to select exactly this action.
    let target: String
    let suggested: Bool
    let source: String
    let title: String
    let safety: Safety
    let bytes: UInt64
    let steps: [Step]
    let warnings: [String]?
    let blockers: [String]?
}

/// Rust `Step` is an internally tagged enum: {"type": "run", "argv": [...]} etc.
struct Step: Decodable, Hashable {
    let type: String
    let argv: [String]?
    let dryRunArgv: [String]?
    let path: String?
}

struct Skipped: Decodable, Identifiable {
    var id: String { target }
    let target: String
    let bytes: UInt64
    let reason: String
}

// MARK: - Screens

enum Screen: String, CaseIterable, Identifiable {
    case overview = "Overview"
    case folders = "Folders"
    case developer = "Developer"
    case projects = "Projects"
    case large = "Large Files"
    case system = "Apps & System"
    var id: String { rawValue }

    var symbol: String {
        switch self {
        case .overview: return "chart.pie"
        case .folders: return "folder"
        case .developer: return "hammer"
        case .projects: return "folder.badge.gearshape"
        case .large: return "doc.zipper"
        case .system: return "gearshape.2"
        }
    }

    /// Rule categories shown on this screen.
    var categories: Set<String> {
        switch self {
        case .developer: return ["package-cache", "docker", "xcode", "ai-models", "browser-runtime", "toolchain"]
        case .system: return ["app-cache", "app-data", "system"]
        default: return []
        }
    }
}

extension Report {
    func rules(for screen: Screen) -> [RuleReport] {
        rules.filter { screen.categories.contains($0.category) && ($0.found || $0.size > 0) }
    }

    func tilde(_ path: String) -> String {
        if path == home { return "~" }
        if path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
        return path
    }

    func daysAgo(_ t: Int64) -> Int64 { max(0, (generatedAt - t) / 86_400) }
}

// MARK: - Formatting

enum Fmt {
    static let bytes: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file // decimal, like Finder
        f.allowsNonnumericFormatting = false
        return f
    }()

    static func size(_ b: UInt64) -> String { bytes.string(fromByteCount: Int64(clamping: b)) }

    static func age(days: Int64) -> String {
        if days == 0 { return "today" }
        if days == 1 { return "yesterday" }
        if days < 60 { return "\(days) days ago" }
        if days < 730 { return "\(days / 30) months ago" }
        return "\(days / 365) years ago"
    }

    static func count(_ n: UInt64) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        return f.string(from: NSNumber(value: n)) ?? "\(n)"
    }
}
