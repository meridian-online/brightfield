// A background-only probe for what App Sandbox grants a child process.
//
// Launched with `open -a Probe.app <folder>`, it is handed the folder through
// macOS's open-documents route (the route a Finder double-click takes), and
// records, one line each, in its container's tmp/probe.log:
//   parent-read      the parent reads <folder>/probe.txt
//   child-nobm       a helper child, given no bookmark, reads it
//   child-bm         a helper child, given a plain bookmark in PROBE_BOOKMARK, reads it
//   grandchild-bm    that child's own child, given the same environment, reads it
//   self-child-bm    the main executable started as its own child, with the bookmark
//   sh               /bin/sh -c 'cat <file>' started by the parent
//   scoped-saved     a security-scoped bookmark is saved for the relaunch case
// Launched with no document, it resolves the saved security-scoped bookmark,
// starts access and records relaunch-read.
import Cocoa

let fm = FileManager.default
let logURL = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("probe.log")

func log(_ line: String) {
    let data = (line + "\n").data(using: .utf8)!
    if let h = try? FileHandle(forWritingTo: logURL) { h.seekToEndOfFile(); h.write(data); h.closeFile() }
    else { try? data.write(to: logURL) }
}

func readFile(_ path: String) -> String {
    do { let s = try String(contentsOfFile: path, encoding: .utf8); return "ok \(s.trimmingCharacters(in: .whitespacesAndNewlines))" }
    catch { return "refused \((error as NSError).localizedDescription)" }
}

func resolvePlain(_ b64: String?) {
    guard let b64, let data = Data(base64Encoded: b64) else { return }
    var stale = false
    _ = try? URL(resolvingBookmarkData: data, options: [.withoutUI], relativeTo: nil, bookmarkDataIsStale: &stale)
}

func run(_ exe: String, _ args: [String], env: [String: String]) -> String {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: exe)
    p.arguments = args
    p.environment = env
    let out = Pipe(); p.standardOutput = out; p.standardError = out
    do { try p.run() } catch { return "spawn-failed \(error.localizedDescription)" }
    p.waitUntilExit()
    let s = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
    return "exit=\(p.terminationStatus) \(s.trimmingCharacters(in: .whitespacesAndNewlines))"
}

let args = CommandLine.arguments
// Child roles print their result to stdout for the parent to log.
if args.count >= 3 && (args[1] == "child" || args[1] == "grandchild") {
    let env = ProcessInfo.processInfo.environment
    resolvePlain(env["PROBE_BOOKMARK"])
    var line = readFile(args[2])
    if args[1] == "child", let helper = env["PROBE_HELPER"] {
        line += " | grandchild: " + run(helper, ["grandchild", args[2]], env: env)
    }
    print(line)
    exit(0)
}

class Delegate: NSObject, NSApplicationDelegate {
    var opened = false
    func application(_ app: NSApplication, open urls: [URL]) {
        opened = true
        for folder in urls { probe(folder) }
        NSApp.terminate(nil)
    }
    func applicationDidFinishLaunching(_ n: Notification) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            if !self.opened { self.relaunch(); NSApp.terminate(nil) }
        }
    }
    func probe(_ folder: URL) {
        let file = folder.appendingPathComponent("probe.txt").path
        log("parent-read \(readFile(file))")
        // Control: a sibling folder nobody granted. A sandbox that is working refuses it.
        let control = folder.deletingLastPathComponent().appendingPathComponent("probe-control/probe.txt").path
        log("parent-control \(readFile(control))")
        let helper = Bundle.main.bundlePath + "/Contents/Helpers/probe-child"
        let base = ["PATH": "/usr/bin:/bin", "PROBE_HELPER": helper]
        log("child-nobm \(run(helper, ["child", file], env: base))")
        log("child-control \(run(helper, ["child", control], env: base))")
        if let plain = try? folder.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
            var env = base; env["PROBE_BOOKMARK"] = plain.base64EncodedString()
            log("child-bm \(run(helper, ["child", file], env: env))")
            log("self-child-bm \(run(Bundle.main.executablePath!, ["grandchild", file], env: env))")
        } else { log("child-bm bookmark-failed") }
        log("sh \(run("/bin/sh", ["-c", "cat '\(file)'"], env: base))")
        log("self-child-nobm \(run(Bundle.main.executablePath!, ["grandchild", file], env: base))")
        // DuckDB's own CLI, keeping its upstream signature, run from three places.
        let sql = "select trim(content) from read_text('\(file)')"
        let duckArgs = ["-noheader", "-list", "-c", sql]
        log("duckdb-in-bundle \(run(Bundle.main.bundlePath + "/Contents/Helpers/duckdb", duckArgs, env: base))")
        log("duckdb-in-granted-folder \(run(folder.appendingPathComponent("duckdb").path, duckArgs, env: base))")
        log("duckdb-in-ungranted-folder \(run(folder.deletingLastPathComponent().appendingPathComponent("probe-control/duckdb").path, duckArgs, env: base))")
        let ctl = folder.deletingLastPathComponent().appendingPathComponent("probe-control/probe.txt").path
        log("duckdb-control \(run(Bundle.main.bundlePath + "/Contents/Helpers/duckdb", ["-noheader", "-list", "-c", "select trim(content) from read_text('\(ctl)')"], env: base))")
        log("duckdb-inherit-signed \(run(Bundle.main.bundlePath + "/Contents/Helpers/duckdb-inherit", duckArgs, env: base))")
        log("duckdb-inherit-control \(run(Bundle.main.bundlePath + "/Contents/Helpers/duckdb-inherit", ["-noheader", "-list", "-c", "select trim(content) from read_text('\(ctl)')"], env: base))")
        let tmp = NSTemporaryDirectory() + "probe-out.txt"
        log("duckdb-writes-granted \(run(Bundle.main.bundlePath + "/Contents/Helpers/duckdb", ["-c", "copy (select 1 as x) to '\(folder.path)/written.csv'"], env: base)) tmp=\(tmp)")
        do {
            let scoped = try folder.bookmarkData(options: [.withSecurityScope], includingResourceValuesForKeys: nil, relativeTo: nil)
            try scoped.write(to: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("scoped.bookmark"))
            log("scoped-saved ok")
        } catch { log("scoped-saved failed \(error.localizedDescription)") }
    }
    func relaunch() {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("scoped.bookmark")
        guard let data = try? Data(contentsOf: url) else { log("relaunch-read no-bookmark"); return }
        var stale = false
        do {
            let folder = try URL(resolvingBookmarkData: data, options: [.withSecurityScope], relativeTo: nil, bookmarkDataIsStale: &stale)
            let started = folder.startAccessingSecurityScopedResource()
            log("relaunch-read started=\(started) stale=\(stale) \(readFile(folder.appendingPathComponent("probe.txt").path))")
            // The child is handed a plain bookmark made from the resolved grant.
            if let plain = try? folder.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
                let helper = Bundle.main.bundlePath + "/Contents/Helpers/probe-child"
                log("relaunch-child-bm \(run(helper, ["child", folder.appendingPathComponent("probe.txt").path], env: ["PROBE_BOOKMARK": plain.base64EncodedString(), "PROBE_HELPER": helper]))")
            }
        } catch { log("relaunch-read failed \(error.localizedDescription)") }
    }
}

let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.prohibited)
app.run()
