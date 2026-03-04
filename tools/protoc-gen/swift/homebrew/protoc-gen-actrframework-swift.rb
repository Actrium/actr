class ProtocGenActrframeworkSwift < Formula
  desc "Swift protoc plugin for generating ACTR framework stubs"
  homepage "https://github.com/actor-rtc/framework-codegen-swift"
  version "0.1.10"
  url "https://github.com/actor-rtc/framework-codegen-swift/releases/download/v#{version}/protoc-gen-actrframework-swift-macos-arm64.zip"
  sha256 "2a1c88a17b73bebc46c76689e8a30b3c43fbd1b4d4e23da2b9e32c6c63ff3844"

  depends_on arch: :arm64
  depends_on macos: :ventura

  def install
    bin.install "protoc-gen-actrframework-swift"
  end

  def caveats
    <<~EOS
      This formula only supports Apple Silicon (arm64).
    EOS
  end
end
