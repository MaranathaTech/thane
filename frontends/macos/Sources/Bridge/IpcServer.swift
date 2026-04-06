import Foundation

/// A lightweight JSON-RPC 2.0 server over a Unix domain socket.
///
/// Listens on the given socket path and dispatches incoming requests
/// to a handler closure. The protocol uses newline-delimited JSON,
/// matching the Rust `thane-ipc` server format.
final class IpcServer {
    private let socketPath: String
    private var fileHandle: FileHandle?
    private var socketFd: Int32 = -1
    private var isRunning = false
    private let handler: @Sendable (JsonRpcRequest) -> JsonRpcResponse

    init(socketPath: String, handler: @escaping @Sendable (JsonRpcRequest) -> JsonRpcResponse) {
        self.socketPath = socketPath
        self.handler = handler
    }

    deinit {
        stop()
    }

    func start() throws {
        // Remove stale socket if it exists.
        unlink(socketPath)

        // Create socket.
        socketFd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard socketFd >= 0 else {
            throw IpcError.socketCreation(errno: errno)
        }

        // Bind to path.
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = socketPath.utf8CString
        guard pathBytes.count <= MemoryLayout.size(ofValue: addr.sun_path) else {
            close(socketFd)
            throw IpcError.pathTooLong
        }
        withUnsafeMutablePointer(to: &addr.sun_path) { sunPath in
            pathBytes.withUnsafeBufferPointer { buf in
                _ = memcpy(sunPath, buf.baseAddress!, buf.count)
            }
        }
        let addrLen = socklen_t(MemoryLayout<sa_family_t>.size + pathBytes.count)
        let bindResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                bind(socketFd, sockPtr, addrLen)
            }
        }
        guard bindResult == 0 else {
            close(socketFd)
            throw IpcError.bindFailed(errno: errno)
        }

        // Set socket permissions (owner-only).
        chmod(socketPath, 0o700)

        // Listen.
        guard listen(socketFd, 5) == 0 else {
            close(socketFd)
            unlink(socketPath)
            throw IpcError.listenFailed(errno: errno)
        }

        isRunning = true
        NSLog("thane: IPC server listening on \(socketPath)")

        // Accept connections on a background queue.
        DispatchQueue.global(qos: .utility).async { [weak self] in
            self?.acceptLoop()
        }
    }

    func stop() {
        isRunning = false
        if socketFd >= 0 {
            close(socketFd)
            socketFd = -1
        }
        unlink(socketPath)
    }

    // MARK: - Private

    private func acceptLoop() {
        while isRunning {
            var clientAddr = sockaddr_un()
            var addrLen = socklen_t(MemoryLayout<sockaddr_un>.size)
            let clientFd = withUnsafeMutablePointer(to: &clientAddr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                    accept(socketFd, sockPtr, &addrLen)
                }
            }
            guard clientFd >= 0 else {
                if isRunning {
                    NSLog("thane: IPC accept error: \(errno)")
                }
                continue
            }

            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.handleClient(fd: clientFd)
            }
        }
    }

    private func handleClient(fd: Int32) {
        let input = FileHandle(fileDescriptor: fd, closeOnDealloc: false)
        defer {
            close(fd)
        }

        // Read all available data line-by-line.
        var buffer = Data()
        while true {
            let chunk = input.availableData
            if chunk.isEmpty { break }
            buffer.append(chunk)

            // Process complete lines.
            while let newlineRange = buffer.range(of: Data([0x0A])) {
                let lineData = buffer.subdata(in: buffer.startIndex..<newlineRange.lowerBound)
                buffer.removeSubrange(buffer.startIndex...newlineRange.lowerBound)

                guard let line = String(data: lineData, encoding: .utf8)?.trimmingCharacters(in: .whitespaces),
                      !line.isEmpty else {
                    continue
                }

                guard let requestData = line.data(using: .utf8),
                      let request = try? JSONDecoder().decode(JsonRpcRequest.self, from: requestData) else {
                    // Send parse error.
                    let errResp = JsonRpcResponse(
                        jsonrpc: "2.0", result: nil,
                        error: JsonRpcError(code: -32700, message: "Parse error", data: nil),
                        id: .null
                    )
                    writeResponse(errResp, to: fd)
                    continue
                }

                let response = handler(request)

                // Only send response if the request had an id.
                if request.id != nil && request.id != .null {
                    writeResponse(response, to: fd)
                }
            }
        }
    }

    private func writeResponse(_ response: JsonRpcResponse, to fd: Int32) {
        guard let data = try? JSONEncoder().encode(response) else { return }
        var toWrite = data
        toWrite.append(0x0A) // newline
        toWrite.withUnsafeBytes { ptr in
            _ = write(fd, ptr.baseAddress!, ptr.count)
        }
    }
}

// MARK: - JSON-RPC Types

struct JsonRpcRequest: Codable {
    let jsonrpc: String
    let method: String
    let params: AnyCodable?
    let id: AnyCodable?
}

struct JsonRpcResponse: Codable {
    let jsonrpc: String
    let result: AnyCodable?
    let error: JsonRpcError?
    let id: AnyCodable?
}

struct JsonRpcError: Codable {
    let code: Int
    let message: String
    let data: AnyCodable?
}

/// A type-erased Codable wrapper for arbitrary JSON values.
enum AnyCodable: Codable, Equatable {
    case null
    case bool(Bool)
    case int(Int)
    case double(Double)
    case string(String)
    case array([AnyCodable])
    case object([String: AnyCodable])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let b = try? container.decode(Bool.self) {
            self = .bool(b)
        } else if let i = try? container.decode(Int.self) {
            self = .int(i)
        } else if let d = try? container.decode(Double.self) {
            self = .double(d)
        } else if let s = try? container.decode(String.self) {
            self = .string(s)
        } else if let arr = try? container.decode([AnyCodable].self) {
            self = .array(arr)
        } else if let obj = try? container.decode([String: AnyCodable].self) {
            self = .object(obj)
        } else {
            self = .null
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let b): try container.encode(b)
        case .int(let i): try container.encode(i)
        case .double(let d): try container.encode(d)
        case .string(let s): try container.encode(s)
        case .array(let arr): try container.encode(arr)
        case .object(let obj): try container.encode(obj)
        }
    }

    /// Extract a string from this value (if it is one).
    var stringValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    /// Extract an object dictionary from this value.
    var objectValue: [String: AnyCodable]? {
        if case .object(let obj) = self { return obj }
        return nil
    }

    /// Extract an int from this value.
    var intValue: Int? {
        if case .int(let i) = self { return i }
        return nil
    }
}

// MARK: - Errors

enum IpcError: Error, CustomStringConvertible {
    case socketCreation(errno: Int32)
    case pathTooLong
    case bindFailed(errno: Int32)
    case listenFailed(errno: Int32)

    var description: String {
        switch self {
        case .socketCreation(let e): return "Failed to create socket: errno \(e)"
        case .pathTooLong: return "Socket path is too long for sockaddr_un"
        case .bindFailed(let e): return "Failed to bind socket: errno \(e)"
        case .listenFailed(let e): return "Failed to listen on socket: errno \(e)"
        }
    }
}
