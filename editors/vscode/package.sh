#!/bin/bash
# package.sh — make homebody-<version>.vsix, the file an editor installs an extension from.
# A .vsix is a zip with a fixed layout; nothing needs downloading to make one.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(node -p "require('$HERE/package.json').version")"
OUT="$HERE/homebody-$VERSION.vsix"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/extension"
cp -R "$HERE/package.json" "$HERE/extension.js" "$HERE/README.md" "$HERE/lib" "$HERE/schemas" "$TMP/extension/"
cp "$HERE/../../LICENSE" "$TMP/extension/LICENSE.txt"
cat > "$TMP/[Content_Types].xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension=".json" ContentType="application/json"/><Default Extension=".js" ContentType="application/javascript"/><Default Extension=".md" ContentType="text/markdown"/><Default Extension=".txt" ContentType="text/plain"/><Default Extension=".vsixmanifest" ContentType="text/xml"/></Types>
XML
cat > "$TMP/extension.vsixmanifest" <<XML
<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011">
  <Metadata>
    <Identity Language="en-US" Id="homebody" Version="$VERSION" Publisher="homebody"/>
    <DisplayName>Homebody</DisplayName>
    <Description xml:space="preserve">See what your AI agents reach, answer Homebody's questions, and open an agent protected, without leaving the editor.</Description>
    <Categories>Other</Categories>
    <Properties><Property Id="Microsoft.VisualStudio.Code.Engine" Value="^1.80.0"/></Properties>
    <License>extension/LICENSE.txt</License>
  </Metadata>
  <Installation><InstallationTarget Id="Microsoft.VisualStudio.Code"/></Installation>
  <Dependencies/>
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true"/>
    <Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true"/>
    <Asset Type="Microsoft.VisualStudio.Services.Content.License" Path="extension/LICENSE.txt" Addressable="true"/>
  </Assets>
</PackageManifest>
XML
rm -f "$OUT"
(cd "$TMP" && zip -q -r "$OUT" "[Content_Types].xml" extension.vsixmanifest extension)
echo "made $OUT ($(du -h "$OUT" | cut -f1))"
