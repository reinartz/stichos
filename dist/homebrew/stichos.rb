class Sticho < Formula
  desc "Local proof/render fidelity toolchain for office documents"
  homepage "https://github.com/reinartz/stichos"
  version "VERSION"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/reinartz/stichos/releases/download/vVERSION/stichos-aarch64-apple-darwin.tar.gz"
      sha256 "MACOS_ARM_SHA256"
    else
      url "https://github.com/reinartz/stichos/releases/download/vVERSION/stichos-x86_64-apple-darwin.tar.gz"
      sha256 "MACOS_X64_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/reinartz/stichos/releases/download/vVERSION/stichos-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "LINUX_ARM_SHA256"
    else
      url "https://github.com/reinartz/stichos/releases/download/vVERSION/stichos-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "LINUX_X64_SHA256"
    end
  end

  def install
    bin.install "stichos"
  end
end
