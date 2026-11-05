import AVFoundation
import Foundation

// op6-cam: capture one still frame from the default camera to a JPEG file.
// Usage: op6-cam <output.jpg>
// Exit codes: 0 ok; 2 no camera; 3 not authorized; 4 capture failed.

let args = CommandLine.arguments
guard args.count == 2 else {
    FileHandle.standardError.write("usage: op6-cam <output.jpg>\n".data(using: .utf8)!)
    exit(64)
}
let outPath = args[1]

switch AVCaptureDevice.authorizationStatus(for: .video) {
case .authorized:
    break
case .notDetermined:
    // Blocking request; over SSH this may never be granted interactively.
    let sem = DispatchSemaphore(value: 0)
    AVCaptureDevice.requestAccess(for: .video) { _ in sem.signal() }
    _ = sem.wait(timeout: .now() + 10)
    if AVCaptureDevice.authorizationStatus(for: .video) != .authorized {
        FileHandle.standardError.write("camera not authorized\n".data(using: .utf8)!)
        exit(3)
    }
default:
    FileHandle.standardError.write("camera not authorized\n".data(using: .utf8)!)
    exit(3)
}

guard let device = AVCaptureDevice.default(for: .video) else {
    FileHandle.standardError.write("no camera device\n".data(using: .utf8)!)
    exit(2)
}

let session = AVCaptureSession()
session.sessionPreset = .photo
do {
    let input = try AVCaptureDeviceInput(device: device)
    guard session.canAddInput(input) else { exit(4) }
    session.addInput(input)
} catch {
    FileHandle.standardError.write("input failed: \(error)\n".data(using: .utf8)!)
    exit(4)
}
let output = AVCapturePhotoOutput()
guard session.canAddOutput(output) else { exit(4) }
session.addOutput(output)

final class Delegate: NSObject, AVCapturePhotoCaptureDelegate {
    var data: Data?
    var done = false
    func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {
        if error == nil {
            self.data = photo.fileDataRepresentation()
        }
        self.done = true
    }
}

let delegate = Delegate()
session.startRunning()
// Let auto-exposure settle briefly.
Thread.sleep(forTimeInterval: 1.5)
output.capturePhoto(with: AVCapturePhotoSettings(), delegate: delegate)
let deadline = Date().addingTimeInterval(10)
while !delegate.done && Date() < deadline {
    Thread.sleep(forTimeInterval: 0.1)
}
session.stopRunning()
guard let data = delegate.data else {
    FileHandle.standardError.write("capture failed: no data\n".data(using: .utf8)!)
    exit(4)
}
do {
    try data.write(to: URL(fileURLWithPath: outPath))
} catch {
    FileHandle.standardError.write("write failed: \(error)\n".data(using: .utf8)!)
    exit(4)
}
print(outPath)
exit(0)
