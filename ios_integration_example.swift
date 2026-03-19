import UIKit
import HostCore // 假设导出的 XCFramework 模块名为 HostCore

class VelloView: UIView {
    private let host = VelloHost()
    private var displayLink: CADisplayLink?
    private var isInitialized = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setup()
    }

    private func setup() {
        // 1. 提取并准备 WASM 模块
        guard let wasmUrl = Bundle.main.url(forResource: "guest", withExtension: "wasm"),
              let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first else {
            return
        }
        
        let destFile = cacheDir.appendingPathComponent("guest.wasm")
        try? FileManager.default.copyItem(at: wasmUrl, to: destFile)
        
        // 2. 初始化引擎（异步或同步预热）
        host.prepareEngine(dataDir: cacheDir.path)
        
        // 3. 设置手势
        let tap = UITapGestureRecognizer(target: self, action: #selector(handleTap))
        self.addGestureRecognizer(tap)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        
        if !isInitialized {
            // 将 UIView 的指针传递给 Rust
            // 在 Rust 端，我们将这个指针包装为 UiKitWindowHandle
            let ptr = unsafeBitCast(self, to: UInt64.self)
            host.initNative(nativeWindowPtr: ptr, dataDir: "", width: UInt32(self.bounds.width), height: UInt32(self.bounds.height))
            
            startAnimation()
            isInitialized = true
        } else {
            host.resizeNative(width: UInt32(self.bounds.width), height: UInt32(self.bounds.height))
        }
    }

    private func startAnimation() {
        displayLink = CADisplayLink(target: self, selector: #selector(tick))
        displayLink?.add(to: .main, forMode: .common)
    }

    @objc private func tick() {
        host.tick()
    }

    @objc private func handleTap(_ gesture: UITapGestureRecognizer) {
        let location = gesture.location(in: self)
        host.onTouch(x: Float(location.x), y: Float(location.y))
    }

    deinit {
        displayLink?.invalidate()
        host.stopNative()
    }
}
