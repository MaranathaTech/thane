import AppKit

/// Protocol for right-side panels that support data refresh.
/// Eliminates type-casting chains in MainWindow panel management.
@MainActor
protocol ReloadablePanel: AnyObject {
    func reload()
}
