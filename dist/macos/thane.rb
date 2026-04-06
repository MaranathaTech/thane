# Homebrew Cask formula for thane
# To install: brew install --cask thane/tap/thane
#
# Tap setup:
#   brew tap thane/tap https://github.com/MaranathaTech/homebrew-tap
#
# This is a template — update the url and sha256 for each release.
cask "thane" do
  version "0.1.0-beta.17"
  sha256 "REPLACE_WITH_SHA256_OF_DMG"

  url "https://github.com/MaranathaTech/thane/releases/download/v#{version}/thane-#{version}.dmg"
  name "thane"
  desc "AI-native terminal workspace manager"
  homepage "https://github.com/MaranathaTech/thane"

  depends_on macos: ">= :ventura"

  app "thane.app"

  binary "#{appdir}/thane.app/Contents/MacOS/thane-cli", target: "thane-cli"

  zap trash: [
    "~/Library/Application Support/thane",
    "~/Library/Caches/thane",
    "~/Library/Preferences/com.thane.app.plist",
  ]
end
