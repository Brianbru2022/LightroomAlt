# Keepframe local build output

This folder is a local hand-off location, not a release channel. Run the following from the repository root to rebuild its installer and checksum from the current source tree:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1
```

Use only the installer named in `BUILD_PROVENANCE.json` and the matching entry in `SHA256SUMS.txt`. The script replaces same-named local artefacts, so an earlier checksum is never presented as the checksum for a newer installer. Installers are deliberately ignored by Git; do not publish this beta build.
