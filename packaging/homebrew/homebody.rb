# Homebrew cask for Homebody. Not submitted anywhere yet: there is no public release to point at.
#
# When a release exists, fill in `version` and `sha256` from it (the release build prints the
# sha256 of the disk image), put this file in a tap (a repository named homebrew-homebody, as
# Casks/homebody.rb), and people can install with:
#
#   brew install --cask <owner>/homebody/homebody
#
# Until the app carries an Apple signature, macOS will ask people to confirm opening it the
# first time (System Settings > Privacy & Security > Open Anyway), installed this way or not.
cask "homebody" do
  version "0.1.0"
  sha256 "REPLACE_WITH_THE_SHA256_OF_THE_DISK_IMAGE"

  url "https://github.com/devesh16145/Homebody/releases/download/v#{version}/Homebody-#{version}-arm64.dmg"
  name "Homebody"
  desc "Keeps AI apps and coding agents to the places you trust, and proves it"
  homepage "https://github.com/devesh16145/Homebody"

  depends_on arch: :arm64
  depends_on macos: ">= :ventura"

  app "Homebody.app"
  binary "#{appdir}/Homebody.app/Contents/MacOS/homebody"

  # What "Turn on protection" sets up needs an administrator to remove, so it is not removed
  # silently here. `homebody setup --print-remove | sudo bash` removes it; see the README.
  uninstall quit: "app.homebody.mac"

  zap trash: [
    "~/Library/Application Support/Homebody",
    "~/Applications/Homebody",
  ]

  caveats <<~EOS
    Open Homebody once and choose "Turn on protection" (it asks for your password once).
    To remove what that set up, before uninstalling:
      homebody setup --print-remove | sudo bash
  EOS
end
