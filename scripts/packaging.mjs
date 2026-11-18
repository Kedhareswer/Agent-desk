// Writes the package manager manifests (Scoop, winget, Chocolatey) and
// SHA256SUMS.txt for a release, from the built files in a dist folder.
//
//   node scripts/packaging.mjs <version> <dist-dir>
//
// <dist-dir> must contain the two release files:
//   Agent-Desk_<version>_x64-setup.exe     NSIS installer (winget, Chocolatey)
//   Agent-Desk_<version>_x64-portable.zip  the bare exe (Scoop)
// Output goes to <dist-dir>/packaging/.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { repoUrl } from "./version.mjs";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

// Identity used by every package manager. Change here only.
const ID = {
  winget: "Kedhareswer.AgentDesk",
  choco: "agent-desk",
  scoop: "agent-desk",
  name: "Agent Desk",
  publisher: "Kedhareswer",
  exe: "agent-desk.exe",
  short: "Voice-first AI assistant that answers through your Claude or ChatGPT subscription.",
  tags: ["ai", "assistant", "claude", "codex", "voice", "overlay", "productivity"],
};

export const files = (version) => ({
  setup: `Agent-Desk_${version}_x64-setup.exe`,
  portable: `Agent-Desk_${version}_x64-portable.zip`,
});

const sha256 = (p) => crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex");

function out(dir, rel, text) {
  const p = path.join(dir, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, text.replace(/\r\n/g, "\n"));
  return p;
}

export function scoop({ version, repo, portableHash }) {
  const dl = `${repo}/releases/download/v$version/Agent-Desk_$version_x64-portable.zip`;
  return {
    version,
    description: ID.short,
    homepage: repo,
    license: "MIT",
    notes: [
      "Agent Desk needs the Claude Code CLI (`claude`) or the Codex CLI (`codex`), signed in.",
      "Start it from the Start menu or run `agent-desk`. Hold Ctrl + Alt and speak.",
    ],
    architecture: {
      "64bit": {
        url: `${repo}/releases/download/v${version}/${files(version).portable}`,
        hash: portableHash,
      },
    },
    bin: ID.exe,
    shortcuts: [[ID.exe, ID.name]],
    checkver: { github: repo },
    autoupdate: {
      architecture: { "64bit": { url: dl, hash: { url: "$baseurl/SHA256SUMS.txt" } } },
    },
  };
}

function wingetManifests({ version, repo, setupHash, notesUrl }) {
  const schema = "1.9.0";
  const head = (type) =>
    `# yaml-language-server: $schema=https://aka.ms/winget-manifest.${type}.${schema}.schema.json\n`;
  const common = `PackageIdentifier: ${ID.winget}\nPackageVersion: ${version}\n`;
  return {
    [`${ID.winget}.yaml`]:
      head("version") + common + `DefaultLocale: en-US\nManifestType: version\nManifestVersion: ${schema}\n`,
    [`${ID.winget}.installer.yaml`]:
      head("installer") +
      common +
      `InstallerType: nullsoft
Scope: user
InstallModes:
  - interactive
  - silent
UpgradeBehavior: install
ReleaseDate: ${new Date().toISOString().slice(0, 10)}
Installers:
  - Architecture: x64
    InstallerUrl: ${repo}/releases/download/v${version}/${files(version).setup}
    InstallerSha256: ${setupHash.toUpperCase()}
ManifestType: installer
ManifestVersion: ${schema}
`,
    [`${ID.winget}.locale.en-US.yaml`]:
      head("defaultLocale") +
      common +
      `PackageLocale: en-US
Publisher: ${ID.publisher}
PublisherUrl: ${repo.replace(/\/[^/]+$/, "")}
PublisherSupportUrl: ${repo}/issues
PackageName: ${ID.name}
PackageUrl: ${repo}
License: MIT
LicenseUrl: ${repo}/blob/HEAD/LICENSE
ShortDescription: ${ID.short}
Description: |-
  Hold Ctrl + Alt, speak, and let go. A small black notch at the edge of the
  screen shows what Agent Desk is doing and grows into the answer. It answers
  through the Claude Code or Codex command-line tool you are already signed in
  to, so there is no API key. Speech is transcribed on your device.
Tags:
${ID.tags.map((t) => `  - ${t}`).join("\n")}
ReleaseNotesUrl: ${notesUrl}
ManifestType: defaultLocale
ManifestVersion: ${schema}
`,
  };
}

function chocolatey({ version, repo, setupHash, notesUrl }) {
  const nuspec = `<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
  <metadata>
    <id>${ID.choco}</id>
    <version>${version}</version>
    <title>${ID.name}</title>
    <authors>${ID.publisher}</authors>
    <owners>${ID.publisher}</owners>
    <projectUrl>${repo}</projectUrl>
    <projectSourceUrl>${repo}</projectSourceUrl>
    <bugTrackerUrl>${repo}/issues</bugTrackerUrl>
    <licenseUrl>${repo}/blob/HEAD/LICENSE</licenseUrl>
    <requireLicenseAcceptance>false</requireLicenseAcceptance>
    <releaseNotes>${notesUrl}</releaseNotes>
    <tags>${ID.tags.join(" ")}</tags>
    <summary>${ID.short}</summary>
    <description>${ID.short} Hold Ctrl + Alt, speak, and let go; the answer appears in a notch at the edge of the screen. Needs the Claude Code CLI or the Codex CLI, signed in.</description>
  </metadata>
  <files>
    <file src="tools\\**" target="tools" />
  </files>
</package>
`;
  const install = `$ErrorActionPreference = 'Stop'
$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  url64bit       = '${repo}/releases/download/v${version}/${files(version).setup}'
  checksum64     = '${setupHash}'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
  softwareName   = '${ID.name}*'
}
Install-ChocolateyPackage @packageArgs
`;
  const uninstall = `$ErrorActionPreference = 'Stop'
$keys = Get-UninstallRegistryKey -SoftwareName '${ID.name}*'
foreach ($key in $keys) {
  $file = ($key.UninstallString -replace '^"([^"]+)".*$', '$1')
  Uninstall-ChocolateyPackage -PackageName $env:ChocolateyPackageName -FileType 'exe' -SilentArgs '/S' -File $file
}
`;
  return {
    [`${ID.choco}.nuspec`]: nuspec,
    "tools/chocolateyinstall.ps1": install,
    "tools/chocolateyuninstall.ps1": uninstall,
  };
}

export function build(version, dist) {
  const repo = repoUrl();
  const names = files(version);
  for (const f of Object.values(names)) {
    if (!fs.existsSync(path.join(dist, f))) throw new Error(`missing ${path.join(dist, f)}`);
  }
  const setupHash = sha256(path.join(dist, names.setup));
  const portableHash = sha256(path.join(dist, names.portable));
  const notesUrl = `${repo}/releases/tag/v${version}`;
  const pkg = path.join(dist, "packaging");
  const ctx = { version, repo, setupHash, portableHash, notesUrl };

  const written = [
    out(dist, "SHA256SUMS.txt", `${setupHash}  ${names.setup}\n${portableHash}  ${names.portable}\n`),
    out(pkg, `scoop/${ID.scoop}.json`, JSON.stringify(scoop(ctx), null, 2) + "\n"),
  ];
  for (const [f, text] of Object.entries(wingetManifests(ctx))) written.push(out(pkg, `winget/${f}`, text));
  for (const [f, text] of Object.entries(chocolatey(ctx))) written.push(out(pkg, `chocolatey/${f}`, text));
  // Chocolatey wants the licence inside the package next to the scripts.
  written.push(out(pkg, "chocolatey/tools/LICENSE.txt", fs.readFileSync(path.join(root, "LICENSE"), "utf8")));
  return written;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [version, dist] = process.argv.slice(2);
  if (!version || !dist) {
    console.error("usage: node scripts/packaging.mjs <version> <dist-dir>");
    process.exit(1);
  }
  for (const p of build(version.replace(/^v/, ""), path.resolve(dist))) console.log(path.relative(process.cwd(), p));
}
