// Small shared views.

import AppKit
import SwiftUI

enum Actions {
    static func reveal(_ path: String) {
        let url = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    }

    static func copy(_ s: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(s, forType: .string)
    }

    /// Apps never add themselves to the Full Disk Access list: the user has to
    /// click + and pick the app. Open the pane and show the app in Finder so it
    /// can be dragged straight into the list.
    static func openFullDiskAccessSettings() {
        if let u = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
            NSWorkspace.shared.open(u)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
            NSWorkspace.shared.activateFileViewerSelecting([Bundle.main.bundleURL])
        }
    }

    /// Shell-quote an argv for display / copying.
    static func shell(_ argv: [String]) -> String {
        argv.map { a in
            a.allSatisfy { $0.isLetter || $0.isNumber || "-_./=:@+".contains($0) } ? a : "'" + a.replacingOccurrences(of: "'", with: "'\\''") + "'"
        }.joined(separator: " ")
    }
}

extension Safety {
    var label: String {
        switch self {
        case .cache: return "Cache"
        case .regenerable: return "Rebuilds"
        case .review: return "Review"
        case .neverAuto: return "Don't delete"
        }
    }

    var help: String {
        switch self {
        case .cache: return "A cache: deleting it only costs download or rebuild time."
        case .regenerable: return "Build output your tools regenerate automatically."
        case .review: return "Worth a look first: a big re-download, or it may hold something you want."
        case .neverAuto: return "SpaceSense shows its size but never cleans it."
        }
    }

    var color: Color {
        switch self {
        case .cache: return .green
        case .regenerable: return .teal
        case .review: return .orange
        case .neverAuto: return .secondary
        }
    }
}

struct SafetyBadge: View {
    let safety: Safety
    var body: some View {
        Text(safety.label)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .foregroundColor(safety.color)
            .background(Capsule().fill(safety.color.opacity(0.15)))
            .help(safety.help)
    }
}

struct Chip: View {
    let text: String
    var color: Color = .secondary
    var symbol: String? = nil
    var body: some View {
        HStack(spacing: 4) {
            if let s = symbol { Image(systemName: s) }
            Text(text)
        }
        .font(.caption)
        .padding(.horizontal, 7)
        .padding(.vertical, 2)
        .foregroundColor(color)
        .background(Capsule().strokeBorder(color.opacity(0.4)))
    }
}

struct SizeText: View {
    let bytes: UInt64
    var font: Font = .body
    var body: some View {
        Text(Fmt.size(bytes)).font(font.monospacedDigit()).lineLimit(1)
    }
}

/// Horizontal bar showing `fraction` of the width.
struct Meter: View {
    let fraction: Double
    var color: Color = .accentColor
    var height: CGFloat = 6
    var body: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.secondary.opacity(0.12))
                Capsule().fill(color).frame(width: max(2, g.size.width * CGFloat(min(1, max(0, fraction)))))
            }
        }
        .frame(height: height)
    }
}

struct Card<Content: View>: View {
    let content: Content
    init(@ViewBuilder _ content: () -> Content) { self.content = content() }
    var body: some View {
        content
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 10).fill(Color(nsColor: .controlBackgroundColor)))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.secondary.opacity(0.15)))
    }
}

struct CommandText: View {
    let argv: [String]
    var body: some View {
        let text = Actions.shell(argv.map { ($0 as NSString).lastPathComponent == $0 ? $0 : ($0 as NSString).lastPathComponent })
        HStack(spacing: 6) {
            Text(text)
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(RoundedRectangle(cornerRadius: 4).fill(Color.secondary.opacity(0.1)))
            Button {
                Actions.copy(text)
            } label: {
                Image(systemName: "doc.on.doc")
            }
            .buttonStyle(.borderless)
            .help("Copy command")
        }
    }
}

struct PathLine: View {
    let path: String
    let display: String
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "folder").foregroundColor(.secondary)
            Text(display).font(.callout).foregroundColor(.secondary).lineLimit(1).truncationMode(.middle)
            Button("Reveal") { Actions.reveal(path) }
                .buttonStyle(.link)
                .font(.callout)
        }
    }
}

struct ScreenHeader: View {
    let title: String
    let subtitle: String
    var total: UInt64? = nil
    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.largeTitle.weight(.semibold))
                Text(subtitle).foregroundColor(.secondary)
            }
            Spacer()
            if let t = total {
                SizeText(bytes: t, font: .title.weight(.semibold))
            }
        }
    }
}
