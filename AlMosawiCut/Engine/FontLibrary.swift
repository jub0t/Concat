import CoreText
import Foundation
import Observation
import UIKit

/// Fonts the user brought in from Files. They live in Documents/Fonts, are
/// registered for this process at launch, and every project can use them.
@MainActor
@Observable
final class FontLibrary {
    static let shared = FontLibrary()

    struct Entry: Identifiable, Hashable {
        var id: String { postScriptName }
        let postScriptName: String
        let displayName: String
        let file: String
    }

    enum FontError: LocalizedError {
        case unreadable(String)
        var errorDescription: String? {
            switch self {
            case .unreadable(let name): "“\(name)” isn't a font AlMosawiCut can read. Use .ttf, .otf or .ttc files."
            }
        }
    }

    private(set) var fonts: [Entry] = []
    /// Title fonts shipped inside the app (the Concat title presets use them).
    private(set) var bundled: [Entry] = []

    nonisolated static let folder: URL = {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let url = docs.appendingPathComponent("Fonts", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    private init() {
        loadBundled()
        load()
    }

    /// Registers the fonts in the app's Fonts folder (SIL Open Font License; their licences sit beside them).
    private func loadBundled() {
        let urls = ["ttf", "otf"].flatMap { ext in
            (Bundle.main.urls(forResourcesWithExtension: ext, subdirectory: "Fonts") ?? [])
                + (Bundle.main.urls(forResourcesWithExtension: ext, subdirectory: nil) ?? [])
        }
        var found: [Entry] = []
        var seen = Set<String>()
        for url in urls {
            Self.register(url)
            for e in Self.entries(in: url) where seen.insert(e.postScriptName).inserted { found.append(e) }
        }
        bundled = found.sorted { $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending }
    }

    /// Registers every saved font file and lists its faces. Safe to call again.
    func load() {
        let files = (try? FileManager.default.contentsOfDirectory(at: Self.folder, includingPropertiesForKeys: nil)) ?? []
        var found: [Entry] = []
        for url in files {
            Self.register(url)
            found += Self.entries(in: url)
        }
        var seen = Set<String>()
        fonts = found
            .filter { seen.insert($0.postScriptName).inserted }
            .sorted { $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending }
    }

    /// Copies a font file in from Files and registers it. Returns the faces it holds.
    @discardableResult
    func importFont(at url: URL) throws -> [Entry] {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        guard !Self.entries(in: url).isEmpty else { throw FontError.unreadable(url.lastPathComponent) }
        let dest = Self.folder.appendingPathComponent(url.lastPathComponent)
        if FileManager.default.fileExists(atPath: dest.path) {
            Self.unregister(dest)
            try FileManager.default.removeItem(at: dest)
        }
        try FileManager.default.copyItem(at: url, to: dest)
        load()
        return Self.entries(in: dest)
    }

    /// Removes the whole file the face came from (a .ttc can hold several faces).
    func remove(_ entry: Entry) {
        let url = Self.folder.appendingPathComponent(entry.file)
        Self.unregister(url)
        try? FileManager.default.removeItem(at: url)
        load()
    }

    func displayName(for postScriptName: String) -> String {
        (fonts + bundled).first { $0.postScriptName == postScriptName }?.displayName ?? postScriptName
    }

    func isInstalled(_ postScriptName: String) -> Bool {
        (fonts + bundled).contains { $0.postScriptName == postScriptName }
    }

    nonisolated private static func register(_ url: URL) {
        // Fails harmlessly when the file is already registered.
        CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
    }

    nonisolated private static func unregister(_ url: URL) {
        CTFontManagerUnregisterFontsForURL(url as CFURL, .process, nil)
    }

    nonisolated private static func entries(in url: URL) -> [Entry] {
        guard let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor] else { return [] }
        return descriptors.compactMap { d in
            guard let ps = CTFontDescriptorCopyAttribute(d, kCTFontNameAttribute) as? String else { return nil }
            let family = CTFontDescriptorCopyAttribute(d, kCTFontFamilyNameAttribute) as? String ?? ""
            let style = CTFontDescriptorCopyAttribute(d, kCTFontStyleNameAttribute) as? String ?? ""
            let name = [family, style].filter { !$0.isEmpty && $0 != "Regular" }.joined(separator: " ")
            return Entry(postScriptName: ps, displayName: name.isEmpty ? ps : name, file: url.lastPathComponent)
        }
    }
}
