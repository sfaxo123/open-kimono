import Foundation
import Vision
import AppKit

// op6-ocr: OCR an image file with Apple Vision; prints recognized text lines.
// Usage: op6-ocr <image>
// Exit codes: 0 ok (text may be empty); 2 image unreadable.

let args = CommandLine.arguments
guard args.count == 2 else {
    FileHandle.standardError.write("usage: op6-ocr <image>\n".data(using: .utf8)!)
    exit(64)
}
let url = URL(fileURLWithPath: args[1])
guard let image = NSImage(contentsOf: url),
      let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let cgImage = bitmap.cgImage else {
    FileHandle.standardError.write("image unreadable\n".data(using: .utf8)!)
    exit(2)
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = false
let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
do {
    try handler.perform([request])
} catch {
    FileHandle.standardError.write("ocr failed: \(error)\n".data(using: .utf8)!)
    exit(1)
}
let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
print(lines.joined(separator: "\n"))
exit(0)
