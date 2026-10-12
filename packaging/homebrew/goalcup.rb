# Template. The release workflow renders it with scripts/render-formula.py, which fills every
# @TOKEN@ from the release's SHA256SUMS, and attaches the result to the release as goalcup.rb.
# That rendered file is what goes into the tap as Formula/goalcup.rb (packaging/README.md).
class Goalcup < Formula
  desc "Record a coding agent's run into a sealed, offline-verifiable trace"
  homepage "https://github.com/agentcommunity/goal-cup-tracer"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "@BASE_URL@/goalcup-@VERSION@-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "@BASE_URL@/goalcup-@VERSION@-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_X86_64_APPLE_DARWIN@"
    end
  end

  on_linux do
    on_arm do
      url "@BASE_URL@/goalcup-@VERSION@-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_AARCH64_UNKNOWN_LINUX_GNU@"
    end
    on_intel do
      url "@BASE_URL@/goalcup-@VERSION@-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_X86_64_UNKNOWN_LINUX_GNU@"
    end
  end

  def install
    # goalcup starts goalcup-daemon from beside itself, and loads the pinned connectors from
    # <exe dir>/../share/goalcup/connectors: here, pkgshare/"connectors".
    bin.install "bin/goalcup", "bin/goalcup-daemon", "bin/goalcup-verify"
    pkgshare.install "share/goalcup/connectors"
    prefix.install "LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.md"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/goalcup --version")
    assert_match version.to_s, shell_output("#{bin}/goalcup-verify --version")

    # The installed goalcup finds the connectors and they are the ones this build pins. doctor
    # exits 1 here because no pinned harness is installed; only the pin checks matter.
    ENV["GOALCUP_CONFIG_DIR"] = (testpath/"goalcup").to_s
    report = JSON.parse(shell_output("#{bin}/goalcup doctor --json || true"))
    pins = report["connectors"].map do |connector|
      connector["checks"].find { |check| check["id"] == "manifest_pin" }&.fetch("level")
    end
    refute_empty pins
    assert_equal ["pass"], pins.uniq
  end
end
