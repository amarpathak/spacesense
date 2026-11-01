// Window layout: sidebar of screens, detail per screen, scanning / error states.

import SwiftUI

struct ContentView: View {
    @EnvironmentObject var model: AppModel
    @EnvironmentObject var scanner: Scanner

    var body: some View {
        NavigationSplitView {
            Sidebar()
                .navigationSplitViewColumnWidth(min: 200, ideal: 220)
        } detail: {
            switch scanner.state {
            case .idle:
                Color.clear
            case .scanning(let p):
                ScanningView(progress: p)
            case .failed(let msg):
                FailedView(message: msg)
            case .done(let report, let plan):
                DetailView(report: report, plan: plan)
                    .sheet(isPresented: $model.showPlan) {
                        PlanSheet(report: report, plan: plan)
                    }
            }
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    scanner.start(root: LaunchOptions.current.root)
                } label: {
                    Label("Rescan", systemImage: "arrow.clockwise")
                }
                .help("Scan again (⌘R)")
                .disabled(scanner.isScanning)
            }
        }
    }
}

struct Sidebar: View {
    @EnvironmentObject var model: AppModel
    @EnvironmentObject var scanner: Scanner

    var body: some View {
        VStack(spacing: 0) {
            List(selection: Binding(get: { Optional(model.screen) }, set: { if let s = $0 { model.screen = s } })) {
                ForEach(Screen.allCases) { s in
                    HStack {
                        Label(s.rawValue, systemImage: s.symbol)
                        Spacer()
                        if let b = badge(s) {
                            Text(Fmt.size(b)).font(.caption.monospacedDigit()).foregroundColor(.secondary)
                        }
                    }
                    .tag(s)
                }
            }
            .listStyle(.sidebar)
            Divider()
            footer.padding(12)
        }
    }

    private func badge(_ s: Screen) -> UInt64? {
        guard case .done(let r, _) = scanner.state else { return nil }
        switch s {
        case .overview, .folders: return nil
        case .developer, .system: return r.rules(for: s).reduce(0) { $0 + $1.size }
        case .projects: return r.projects.reduce(0) { $0 + $1.artifactBytes }
        case .large: return r.largeFiles.reduce(0) { $0 + $1.size }
        }
    }

    @ViewBuilder private var footer: some View {
        switch scanner.state {
        case .done(let r, _):
            VStack(alignment: .leading, spacing: 2) {
                Text("Scanned \(r.tilde(r.root))").font(.caption)
                Text("\(Fmt.count(r.scan.files)) files in \(String(format: "%.0f", scanner.lastDuration > 0 ? scanner.lastDuration : Double(r.scan.elapsedMs) / 1000)) s")
                    .font(.caption).foregroundColor(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .scanning:
            HStack {
                ProgressView().controlSize(.small)
                Text("Scanning…").font(.caption)
                Spacer()
            }
        default:
            EmptyView()
        }
    }
}

struct DetailView: View {
    @EnvironmentObject var model: AppModel
    let report: Report
    let plan: Plan

    var body: some View {
        switch model.screen {
        case .overview: OverviewView(report: report, plan: plan)
        case .folders: FolderBrowser(report: report)
        case .developer: RulesView(report: report, screen: .developer)
        case .projects: ProjectsView(report: report)
        case .large: LargeFilesView(report: report)
        case .system: RulesView(report: report, screen: .system)
        }
    }
}

struct ScanningView: View {
    let progress: ScanProgress

    var phaseText: String {
        switch progress.phase {
        case 1: return "Measuring caches and tools…\nNothing is changed."
        case 2: return "Checking git repositories (branch, uncommitted changes, ignored folders)…\nNothing is changed."
        default: return "Reading your home folder.\nNothing is changed."
        }
    }
    var body: some View {
        VStack(spacing: 18) {
            ProgressView().controlSize(.large)
            SizeText(bytes: progress.bytes, font: .system(size: 44, weight: .semibold))
            Text("\(Fmt.count(progress.files)) files in \(Fmt.count(progress.dirs)) folders")
                .foregroundColor(.secondary)
                .font(.body.monospacedDigit())
            Text(phaseText)
                .multilineTextAlignment(.center)
                .font(.callout)
                .foregroundColor(.secondary)
            if progress.waitingForPermission {
                VStack(spacing: 10) {
                    Label("Some folders are slow to open", systemImage: "hourglass")
                        .font(.headline)
                    Text("Usually macOS is asking whether SpaceSense may read a folder (Desktop, Documents, Downloads or other apps' data): the scan continues once you answer; if you decline, that folder is skipped. Giving SpaceSense Full Disk Access avoids these dialogs. SpaceSense never downloads iCloud files to measure them.")
                        .font(.callout)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                    Button("Open Full Disk Access Settings…") { Actions.openFullDiskAccessSettings() }
                }
                .padding(16)
                .frame(maxWidth: 520)
                .background(RoundedRectangle(cornerRadius: 10).fill(Color.orange.opacity(0.1)))
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct FailedView: View {
    @EnvironmentObject var scanner: Scanner
    let message: String
    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle").font(.largeTitle).foregroundColor(.orange)
            Text("The scan didn't finish").font(.title2)
            Text(message).foregroundColor(.secondary).textSelection(.enabled).multilineTextAlignment(.center)
            Button("Try again") { scanner.start(root: LaunchOptions.current.root) }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
