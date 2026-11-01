// Runs the Rust scan (ss_report) off the main thread and publishes progress.
// Nothing runs while idle: no timers, no polling (PLAN.md §3.3, FanBoost lesson).

import Foundation

struct ScanProgress: Equatable {
    var dirs: UInt64 = 0
    var files: UInt64 = 0
    var bytes: UInt64 = 0
    /// 0 reading folders, 1 measuring caches, 2 checking git repositories.
    var phase: UInt32 = 0
    /// No folder read for a few seconds while reading: almost always macOS
    /// waiting for the user to answer a privacy (TCC) dialog, which blocks the read.
    var waitingForPermission = false
}

final class Scanner: ObservableObject {
    enum State {
        case idle
        case scanning(ScanProgress)
        case done(Report, Plan)
        case failed(String)
    }

    @Published private(set) var state: State = .idle
    @Published private(set) var lastDuration: TimeInterval = 0
    /// Non-nil while results stay on screen and a new scan runs behind them
    /// (after cleaning, or Rescan when results are already shown).
    @Published private(set) var refreshing: ScanProgress?

    var isScanning: Bool {
        if refreshing != nil { return true }
        if case .scanning = state { return true }
        return false
    }

    var hasResults: Bool {
        if case .done = state { return true }
        return false
    }

    /// Box passed through the C callback's `ctx` pointer.
    private final class Sink {
        weak var scanner: Scanner?
        var lastDirs: UInt64 = 0
        var lastChange = Date()
        init(_ s: Scanner) { scanner = s }
    }

    /// Scan. When results are already shown, they stay visible and are
    /// replaced when the new scan finishes.
    func start(root: String? = nil) {
        guard !isScanning else { return }
        let background = hasResults
        if background {
            refreshing = ScanProgress()
        } else {
            state = .scanning(ScanProgress())
        }
        let sink = Unmanaged.passRetained(Sink(self))
        let started = Date()
        // Without this, App Nap throttles a scan started while the window isn't
        // frontmost (measured: > 80 s instead of ~25 s). Ended as soon as the scan is.
        let activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiated, .latencyCritical], reason: "Scanning disk usage")
        DispatchQueue.global(qos: .userInitiated).async {
            let cb: ss_progress_cb = { ctx, dirs, files, bytes, phase in
                guard let ctx = ctx else { return }
                let sink = Unmanaged<Sink>.fromOpaque(ctx).takeUnretainedValue()
                DispatchQueue.main.async {
                    guard let s = sink.scanner, s.isScanning else { return }
                    if dirs != sink.lastDirs {
                        sink.lastDirs = dirs
                        sink.lastChange = Date()
                    }
                    let stalled = phase == 0 && dirs > 0 && Date().timeIntervalSince(sink.lastChange) > 4
                    let p = ScanProgress(dirs: dirs, files: files, bytes: bytes, phase: phase,
                                         waitingForPermission: stalled)
                    if s.refreshing != nil {
                        s.refreshing = p
                    } else {
                        s.state = .scanning(p)
                    }
                }
            }
            let raw: UnsafeMutablePointer<CChar>?
            if let root = root {
                raw = root.withCString { ss_report($0, cb, sink.toOpaque()) }
            } else {
                raw = ss_report(nil, cb, sink.toOpaque())
            }
            let json = raw.map { String(cString: $0) } ?? "{\"error\":\"no result\"}"
            ss_free(raw)
            let result = Scanner.decode(Data(json.utf8))
            DispatchQueue.main.async {
                sink.release()
                ProcessInfo.processInfo.endActivity(activity)
                self.lastDuration = Date().timeIntervalSince(started)
                // A failed background refresh keeps the previous results visible.
                if self.refreshing == nil || { if case .done = result { return true }; return false }() {
                    self.state = result
                }
                self.refreshing = nil
                print(String(format: "spacesense: scan finished in %.1f s", self.lastDuration))
                fflush(stdout)
            }
        }
    }

    /// Show a saved payload (`spacesense report --json --plan`) instead of scanning.
    func load(file: URL) {
        do {
            state = Scanner.decode(try Data(contentsOf: file))
        } catch {
            state = .failed("\(file.path): \(error.localizedDescription)")
        }
    }

    static func decode(_ data: Data) -> State {
        struct ErrorPayload: Decodable { let error: String }
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        if let e = try? d.decode(ErrorPayload.self, from: data) {
            return .failed(e.error)
        }
        do {
            let p = try d.decode(AppPayload.self, from: data)
            return .done(p.report, p.plan)
        } catch {
            return .failed("Couldn't read the scan result: \(error)")
        }
    }
}

/// Space macOS can free on its own (purgeable), from Foundation. Volume-level only:
/// it is never attributed to files (PLAN.md §3.1).
struct VolumeCapacity {
    let available: Int64
    let importantUsage: Int64
    var purgeable: Int64 { max(0, importantUsage - available) }

    static func read(_ path: String) -> VolumeCapacity? {
        let url = URL(fileURLWithPath: path)
        guard let v = try? url.resourceValues(forKeys: [
            .volumeAvailableCapacityKey, .volumeAvailableCapacityForImportantUsageKey,
        ]), let a = v.volumeAvailableCapacity, let i = v.volumeAvailableCapacityForImportantUsage
        else { return nil }
        return VolumeCapacity(available: Int64(a), importantUsage: i)
    }
}
