import Foundation
import Observation

/// Projects on disk: Documents/Projects/<id>/project.json plus a media folder.
@MainActor
@Observable
final class ProjectStore {
    private(set) var projects: [Project] = []
    /// Saved templates: projects whose main clips are slots to fill with new footage.
    private(set) var templates: [Project] = []

    nonisolated static let root: URL = {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let url = docs.appendingPathComponent("Projects", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    nonisolated static func folder(for id: UUID) -> URL {
        root.appendingPathComponent(id.uuidString, isDirectory: true)
    }

    nonisolated static func mediaFolder(for id: UUID) -> URL {
        let url = folder(for: id).appendingPathComponent("media", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    init() {
        reload()
        reloadTemplates()
    }

    // MARK: Templates

    nonisolated static let templatesRoot: URL = {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let url = docs.appendingPathComponent("Templates", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    func reloadTemplates() {
        let dirs = (try? FileManager.default.contentsOfDirectory(at: Self.templatesRoot, includingPropertiesForKeys: nil)) ?? []
        templates = dirs.compactMap { dir in
            guard let data = try? Data(contentsOf: dir.appendingPathComponent("template.json")) else { return nil }
            return Self.decode(data)
        }
        .sorted { $0.createdAt > $1.createdAt }
    }

    /// Saves the project (its media too) as a template.
    func saveTemplate(from project: Project, name: String) {
        var t = project
        t.id = UUID()
        t.name = name
        t.createdAt = Date()
        let folder = Self.templatesRoot.appendingPathComponent(t.id.uuidString, isDirectory: true)
        let fm = FileManager.default
        try? fm.createDirectory(at: folder, withIntermediateDirectories: true)
        try? fm.copyItem(at: Self.mediaFolder(for: project.id), to: folder.appendingPathComponent("media"))
        try? JSONEncoder().encode(t).write(to: folder.appendingPathComponent("template.json"), options: .atomic)
        reloadTemplates()
    }

    func deleteTemplate(_ template: Project) {
        try? FileManager.default.removeItem(at: Self.templatesRoot.appendingPathComponent(template.id.uuidString))
        reloadTemplates()
    }

    /// A new project from a template, with the template's media copied in.
    func project(fromTemplate template: Project) -> Project {
        var p = template
        p.id = UUID()
        p.createdAt = Date()
        let fm = FileManager.default
        try? fm.createDirectory(at: Self.folder(for: p.id), withIntermediateDirectories: true)
        let source = Self.templatesRoot.appendingPathComponent(template.id.uuidString).appendingPathComponent("media")
        try? fm.copyItem(at: source, to: Self.folder(for: p.id).appendingPathComponent("media"))
        save(p)
        return p
    }

    func reload() {
        let fm = FileManager.default
        let dirs = (try? fm.contentsOfDirectory(at: Self.root, includingPropertiesForKeys: nil)) ?? []
        projects = dirs.compactMap { dir in
            guard let data = try? Data(contentsOf: dir.appendingPathComponent("project.json")) else { return nil }
            return Self.decode(data)
        }
        .sorted { $0.modifiedAt > $1.modifiedAt }
    }

    func create(named name: String) -> Project {
        let project = Project(name: name)
        _ = Self.mediaFolder(for: project.id)
        save(project)
        return project
    }

    func save(_ project: Project) {
        var project = project
        project.modifiedAt = Date()
        let encoder = JSONEncoder()
        encoder.outputFormatting = .prettyPrinted
        let folder = Self.folder(for: project.id)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try? encoder.encode(project).write(to: folder.appendingPathComponent("project.json"), options: .atomic)
        if let i = projects.firstIndex(where: { $0.id == project.id }) {
            projects[i] = project
        } else {
            projects.insert(project, at: 0)
        }
        projects.sort { $0.modifiedAt > $1.modifiedAt }
    }

    func delete(_ project: Project) {
        try? FileManager.default.removeItem(at: Self.folder(for: project.id))
        projects.removeAll { $0.id == project.id }
    }

    func duplicate(_ project: Project) {
        var copy = project
        copy.id = UUID()
        copy.name = project.name + " copy"
        copy.createdAt = Date()
        let fm = FileManager.default
        try? fm.createDirectory(at: Self.folder(for: copy.id), withIntermediateDirectories: true)
        try? fm.copyItem(at: Self.mediaFolder(for: project.id), to: Self.folder(for: copy.id).appendingPathComponent("media"))
        save(copy)
    }

    func rename(_ project: Project, to name: String) {
        var p = project
        p.name = name
        save(p)
    }

    // MARK: Older files

    /// Decodes a project written by any earlier version: fields it lacks take
    /// their defaults, and the single music track becomes an audio item.
    nonisolated static func decode(_ data: Data) -> Project? {
        guard var stored = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        if let music = stored["music"] as? [String: Any], stored["audios"] == nil {
            var audio = music
            audio["kind"] = "music"
            audio["start"] = 0
            audio["trimStart"] = 0
            audio["trimEnd"] = music["sourceDuration"] ?? 0
            audio["fadeOut"] = 1
            stored["audios"] = [audio]
        }
        let merged = merge(stored, template)
        guard let fixed = try? JSONSerialization.data(withJSONObject: merged) else { return nil }
        let decoder = JSONDecoder()
        return try? decoder.decode(Project.self, from: fixed)
    }

    /// A project with one of everything, encoded, so every array has an element shape to fill from.
    nonisolated private static let template: Any = {
        var p = Project()
        p.clips = [Clip()]
        p.overlays = [OverlayItem()]
        p.texts = [TextOverlay()]
        p.audios = [AudioItem()]
        p.clips[0].effects = [AppliedEffect()]
        p.clips[0].keyframes = [Keyframe(time: 0, value: Transform())]
        let data = (try? JSONEncoder().encode(p)) ?? Data()
        return (try? JSONSerialization.jsonObject(with: data)) ?? [:]
    }()

    nonisolated private static func merge(_ stored: Any, _ defaults: Any) -> Any {
        if let s = stored as? [String: Any], let d = defaults as? [String: Any] {
            var out = s
            for (key, value) in d {
                if let existing = s[key] {
                    out[key] = merge(existing, value)
                } else {
                    // The template's arrays hold sample elements; a missing list starts empty.
                    out[key] = value is [Any] ? [Any]() : value
                }
            }
            return out
        }
        if let s = stored as? [Any], let d = defaults as? [Any], let shape = d.first {
            return s.map { merge($0, shape) }
        }
        return stored
    }
}
