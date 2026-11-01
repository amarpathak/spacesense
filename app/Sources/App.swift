// SpaceSense app entry point.
//
// Command-line options (for development):
//   --load FILE      show a saved `spacesense report --json --plan` instead of scanning
//   --root PATH      scan PATH instead of the home folder
//   --snapshot DIR   render every screen of the real window to DIR/<screen>.png, then quit

import AppKit
import Combine
import SwiftUI

final class AppModel: ObservableObject {
    static let shared = AppModel()
    let scanner = Scanner()
    let cleaner = Cleaner()
    @Published var screen: Screen = .overview
    private var sub: Any?

    init() {
        // After every scan, refresh the list of things that *could* be cleaned
        // (nothing selected).
        sub = scanner.$state.sink { [weak self] st in
            if case .done = st {
                DispatchQueue.main.async { self?.cleaner.loadCandidates() }
            }
        }
    }
    /// Folder shown on the Folders screen (nil = scan root).
    @Published var folderPath: String?

    func browse(_ path: String) {
        folderPath = path
        screen = .folders
    }
}

struct LaunchOptions {
    var load: URL?
    var root: String?
    var snapshot: URL?

    static let current: LaunchOptions = {
        var o = LaunchOptions()
        var it = CommandLine.arguments.dropFirst().makeIterator()
        while let a = it.next() {
            switch a {
            case "--load": o.load = it.next().map { URL(fileURLWithPath: $0) }
            case "--root": o.root = it.next()
            case "--snapshot": o.snapshot = it.next().map { URL(fileURLWithPath: $0, isDirectory: true) }
            default: break
            }
        }
        return o
    }()
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var sigterm: DispatchSourceSignal?

    func applicationDidFinishLaunching(_ note: Notification) {
        // Quit cleanly on SIGTERM (FanBoost lesson).
        signal(SIGTERM, SIG_IGN)
        let src = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        src.setEventHandler { AppDelegate.quitNow() }
        src.resume()
        sigterm = src

        let opts = LaunchOptions.current
        let model = AppModel.shared
        if let f = opts.load {
            model.scanner.load(file: f)
        } else {
            model.scanner.start(root: opts.root)
        }
        if let dir = opts.snapshot {
            Snapshotter.run(into: dir, model: model)
        }
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Close sheets first (an open sheet blocks terminate), then quit; force
    /// exit if something still holds the app open. Nothing is ever mid-write:
    /// the app is read-only.
    static func quitNow() {
        for w in NSApp.windows {
            if let sheet = w.attachedSheet { w.endSheet(sheet) }
        }
        NSApp.terminate(nil)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { exit(0) }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

@main
struct SpaceSenseApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @ObservedObject private var model = AppModel.shared

    var body: some Scene {
        WindowGroup("SpaceSense") {
            ContentView()
                .environmentObject(model)
                .environmentObject(model.scanner)
                .environmentObject(model.cleaner)
                .frame(minWidth: 900, minHeight: 600)
        }
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(replacing: .newItem) {}
            CommandMenu("Scan") {
                Button("Rescan") { model.scanner.start(root: LaunchOptions.current.root) }
                    .keyboardShortcut("r")
                    .disabled(model.scanner.isScanning)
            }
        }
    }
}

/// Renders each screen of the live window (real AppKit controls, unlike
/// ImageRenderer) to PNG files. Used to check layout without Screen Recording.
enum Snapshotter {
    static func run(into dir: URL, model: AppModel) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var steps: [(String, () -> Void)] = Screen.allCases.map { s in
            (s.rawValue, { model.screen = s })
        }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        for sub in ["Library", "Library/Containers", "Library/Application Support", "Library/Caches"] {
            steps.append(("Folders " + sub.replacingOccurrences(of: "/", with: "-"), { model.browse(home + "/" + sub) }))
        }
        steps.append(("Clean-choose", { model.screen = .overview; model.cleaner.open() }))
        // Only shows the confirmation screen; nothing is run.
        steps.append(("Clean-confirm", { model.cleaner.open(with: ["npm-cache", "yarn-v1-cache", "docker-desktop"]) }))
        steps.append(("Folders-Chrome", { model.cleaner.showSheet = false; model.browse(home + "/Library/Application Support/Google/Chrome") }))
        steps.append(("Clean-confirm-folder", {
            model.cleaner.open(with: ["chrome-ai-model", "folder:" + home + "/Library/Caches/com.spotify.client"])
        }))
        func next(_ i: Int) {
            guard i < steps.count else {
                AppDelegate.quitNow()
                return
            }
            steps[i].1()
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                capture(name: steps[i].0, dir: dir)
                next(i + 1)
            }
        }
        // Wait for the window; during a live scan, capture the progress screen
        // and then wait for the result.
        func whenDone(_ f: @escaping () -> Void) {
            if model.scanner.isScanning {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { whenDone(f) }
            } else {
                f()
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
            NSApp.windows.first?.setContentSize(NSSize(width: 1180, height: 820))
            if model.scanner.isScanning {
                DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) {
                    capture(name: "Scanning", dir: dir)
                    whenDone { next(0) }
                }
            } else {
                next(0)
            }
        }
    }

    static func capture(name: String, dir: URL) {
        // The plan is a sheet: capture the frontmost window that has content.
        let win = NSApp.windows.filter { $0.isVisible }.last
        guard let view = win?.contentView?.superview ?? win?.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds)
        else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        let file = dir.appendingPathComponent(name.replacingOccurrences(of: " ", with: "-") + ".png")
        try? rep.representation(using: .png, properties: [:])?.write(to: file)
    }
}
