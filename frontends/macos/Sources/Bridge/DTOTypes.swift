// DTOTypes.swift — Data transfer objects shared across the macOS frontend.
// Extracted from RustBridge.swift to reduce file size and improve organization.

import Foundation

// MARK: - DTO types (match UniFFI-generated structs)
//
// These mirror the bridge's data transfer types. When UniFFI bindings are
// generated, replace these with direct use of the generated types or add
// conversion extensions.

struct WorkspaceInfoDTO {
    let id: String
    let title: String
    let cwd: String
    let tag: String?
    let paneCount: UInt64
    let panelCount: UInt64
    let unreadNotifications: UInt64
}

extension WorkspaceInfoDTO {
    /// Create a copy with selectively updated fields. Eliminates repeated full-struct reconstruction.
    func updating(title: String? = nil, cwd: String? = nil, tag: String?? = nil,
                  paneCount: UInt64? = nil, panelCount: UInt64? = nil,
                  unreadNotifications: UInt64? = nil) -> WorkspaceInfoDTO {
        WorkspaceInfoDTO(
            id: id,
            title: title ?? self.title,
            cwd: cwd ?? self.cwd,
            tag: tag ?? self.tag,
            paneCount: paneCount ?? self.paneCount,
            panelCount: panelCount ?? self.panelCount,
            unreadNotifications: unreadNotifications ?? self.unreadNotifications
        )
    }
}

struct PanelInfoDTO {
    let id: String
    let panelType: PanelTypeDTO
    let title: String
    let location: String
    let hasUnread: Bool
}

struct SplitResultDTO {
    let paneId: String
    let panelId: String
}

struct NotificationInfoDTO {
    let id: String
    let panelId: String
    let title: String
    let body: String
    let urgency: NotifyUrgencyDTO
    let timestamp: String
    let read: Bool
}

struct QueueEntryInfoDTO {
    let id: String
    let content: String
    let workspaceId: String?
    let priority: Int32
    let status: QueueEntryStatusDTO
    let createdAt: String
    let startedAt: String?
    let completedAt: String?
    let error: String?
    let inputTokens: UInt64
    let outputTokens: UInt64
    let cacheReadTokens: UInt64
    let cacheWriteTokens: UInt64
    let estimatedCostUsd: Double
    /// The model Claude Code selected for this task (e.g. "claude-sonnet-4-5-20250514").
    let model: String?
}

struct SessionInfoDTO {
    let restored: Bool
    let workspaceCount: UInt64
}

struct TokenLimitsDTO {
    let planName: String
    let hasCaps: Bool
    /// "utilization" or "dollar" — controls primary metric shown in UI.
    let displayMode: String
    let fiveHourUtilization: Double?
    let fiveHourResetsAt: String?
    let sevenDayUtilization: Double?
    let sevenDayResetsAt: String?
}

struct ProjectCostDTO: Equatable {
    let sessionCostUsd: Double
    let sessionInputTokens: UInt64
    let sessionOutputTokens: UInt64
    let sessionCacheReadTokens: UInt64
    let sessionCacheWriteTokens: UInt64
    let alltimeCostUsd: Double
    let alltimeInputTokens: UInt64
    let alltimeOutputTokens: UInt64
    let alltimeCacheReadTokens: UInt64
    let alltimeCacheWriteTokens: UInt64
    let sessionCount: UInt64
    /// Plan display name (e.g. "Pro", "Max (5x)", "Enterprise").
    let planName: String
    /// "utilization" or "dollar".
    let displayMode: String
    /// 5-hour utilization percentage (0-100), if available.
    let fiveHourUtilization: Double?
    /// 7-day utilization percentage (0-100), if available.
    let sevenDayUtilization: Double?

    static let zero = ProjectCostDTO(
        sessionCostUsd: 0, sessionInputTokens: 0, sessionOutputTokens: 0,
        sessionCacheReadTokens: 0, sessionCacheWriteTokens: 0,
        alltimeCostUsd: 0, alltimeInputTokens: 0, alltimeOutputTokens: 0,
        alltimeCacheReadTokens: 0, alltimeCacheWriteTokens: 0, sessionCount: 0,
        planName: "Pro", displayMode: "dollar",
        fiveHourUtilization: nil, sevenDayUtilization: nil
    )
}

enum SplitOrientationDTO {
    case horizontal
    case vertical
}

/// A binary tree representing the split layout for a workspace.
indirect enum SplitNode {
    case leaf(PanelInfoDTO)
    case split(SplitOrientationDTO, SplitNode, SplitNode)

    /// Find a leaf by panel ID and replace it with a new split node.
    func replacing(panelId: String, with newNode: SplitNode) -> SplitNode {
        switch self {
        case .leaf(let panel):
            if panel.id == panelId { return newNode }
            return self
        case .split(let orientation, let first, let second):
            return .split(orientation,
                          first.replacing(panelId: panelId, with: newNode),
                          second.replacing(panelId: panelId, with: newNode))
        }
    }

    /// Remove a leaf by panel ID, returning the sibling if this was a split.
    func removing(panelId: String) -> SplitNode? {
        switch self {
        case .leaf(let panel):
            return panel.id == panelId ? nil : self
        case .split(let orientation, let first, let second):
            let newFirst = first.removing(panelId: panelId)
            let newSecond = second.removing(panelId: panelId)
            if newFirst == nil { return newSecond }
            if newSecond == nil { return newFirst }
            return .split(orientation, newFirst!, newSecond!)
        }
    }

    /// Collect all panels in the tree.
    var allPanels: [PanelInfoDTO] {
        switch self {
        case .leaf(let panel): return [panel]
        case .split(_, let first, let second): return first.allPanels + second.allPanels
        }
    }

    /// Serialize to a JSON-compatible dictionary.
    func toDict() -> [String: Any] {
        switch self {
        case .leaf(let panel):
            return [
                "type": "leaf",
                "panelId": panel.id,
                "panelType": panel.panelType == .terminal ? "terminal" : "browser",
                "location": panel.location,
            ]
        case .split(let orientation, let first, let second):
            return [
                "type": "split",
                "orientation": orientation == .horizontal ? "horizontal" : "vertical",
                "first": first.toDict(),
                "second": second.toDict(),
            ]
        }
    }

    /// Deserialize from a JSON dictionary.
    static func fromDict(_ dict: [String: Any], panelCwds: [String: String]) -> SplitNode? {
        guard let type = dict["type"] as? String else { return nil }

        if type == "leaf" {
            let panelId = dict["panelId"] as? String ?? UUID().uuidString
            let panelTypeStr = dict["panelType"] as? String ?? "terminal"
            let location = dict["location"] as? String ?? "/"
            let cwd = panelCwds[panelId] ?? location
            let panelType: PanelTypeDTO = panelTypeStr == "browser" ? .browser : .terminal
            let panel = PanelInfoDTO(
                id: panelId, panelType: panelType, title: "Terminal",
                location: cwd, hasUnread: false
            )
            return .leaf(panel)
        } else if type == "split" {
            let orientationStr = dict["orientation"] as? String ?? "horizontal"
            let orientation: SplitOrientationDTO = orientationStr == "vertical" ? .vertical : .horizontal
            guard let firstDict = dict["first"] as? [String: Any],
                  let secondDict = dict["second"] as? [String: Any],
                  let first = fromDict(firstDict, panelCwds: panelCwds),
                  let second = fromDict(secondDict, panelCwds: panelCwds) else { return nil }
            return .split(orientation, first, second)
        }
        return nil
    }
}

enum PanelTypeDTO {
    case terminal
    case browser
}

enum NotifyUrgencyDTO {
    case low
    case normal
    case critical
}

enum QueueEntryStatusDTO {
    case queued
    case running
    case pausedTokenLimit
    case pausedByUser
    case completed
    case failed
    case cancelled
}

struct AuditEventInfoDTO {
    let id: String
    let timestamp: String
    let workspaceId: String
    let panelId: String?
    let eventType: String
    let severity: AuditSeverityDTO
    let description: String
    let metadataJson: String
    let agentName: String?
}

enum AuditSeverityDTO {
    case info
    case warning
    case alert
    case critical

    var ordinal: Int {
        switch self {
        case .info: return 0
        case .warning: return 1
        case .alert: return 2
        case .critical: return 3
        }
    }

    var label: String {
        switch self {
        case .info: return "Info"
        case .warning: return "Warning"
        case .alert: return "Alert"
        case .critical: return "Critical"
        }
    }
}

struct SandboxInfoDTO {
    let enabled: Bool
    let rootDir: String
    let readOnlyPaths: [String]
    let readWritePaths: [String]
    let deniedPaths: [String]
    let allowNetwork: Bool
    let maxOpenFiles: UInt64?
    let maxWriteBytes: UInt64?
    let maxCpuSeconds: UInt64?
    let enforcement: EnforcementLevelDTO
}

enum EnforcementLevelDTO {
    case permissive
    case enforcing
    case strict

    #if canImport(thane_bridgeFFI)
    init(from bridge: BridgeEnforcementLevel) {
        switch bridge {
        case .permissive: self = .permissive
        case .enforcing: self = .enforcing
        case .strict: self = .strict
        }
    }
    #endif
}

struct ConfigEntryDTO {
    let key: String
    let value: String
}

/// Per-panel location info for sidebar display.
struct PanelLocationInfo {
    let cwd: String
    let gitBranch: String?
    let gitDirty: Bool
}

/// A recently closed workspace entry for history tracking.
struct ClosedWorkspaceDTO {
    let id: String
    let title: String
    let cwd: String
    let tag: String?
    let closedAt: String
}
