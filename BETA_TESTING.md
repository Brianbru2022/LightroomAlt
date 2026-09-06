# Invitation-only beta procedure

Use this unsigned build only with 5–10 named testers and disposable copies of 500–2,000 photographs. Send each tester the installer SHA-256 and explain the expected SmartScreen warning. Do not publish a general download.

Each tester should complete: Copy import, restart, timeline and tag browsing, Keep/Discard triage, Trash/Restore, map browsing and manual GPS placement, recipe generation, external PNG/prompt export, returned-edit import and version comparison. Optional local GPU editing is restricted to technically confident testers with compatible hardware.

Stop testing immediately for any case where neither the original source nor a verified managed copy exists. Treat every uncertainty about an original’s location as a priority defect. Ask testers to use Settings & Library Health to export diagnostics explicitly; do not collect their library or prompts automatically.

Record Windows version, display scaling, collection composition, timings, failures, whether help was required and whether the tester returned for at least three sessions. Closed beta succeeds only when the criteria in `RELEASE_GATES.md` are evidenced.

## Installer verification and SmartScreen

Before running the invitation-only installer, confirm that its filename is the one in `release\BUILD_PROVENANCE.json`, then compare its SHA-256 with `release\SHA256SUMS.txt`:

```powershell
Get-FileHash -Algorithm SHA256 .\Keepframe_0.2.0-beta.1_x64-setup.exe
```

This beta is unsigned, so Windows may show a Microsoft Defender SmartScreen warning. Only named testers who received the checksum directly should choose **More info**, verify the publisher is unknown as expected, verify the filename/checksum, and then choose **Run anyway**. Do not disable SmartScreen or other Windows security controls. Public builds must be signed.

## Clean-VM install, upgrade and uninstall checklist

This is a manual external-release gate; it has not been passed by local build or automated tests. Use a fresh Windows 10/11 VM (or equivalent clean Windows profile), and record the Windows version, installer filename and SHA-256 from `release\BUILD_PROVENANCE.json`.

1. Verify the installer filename and SHA-256 against `release\SHA256SUMS.txt`, then install the current unsigned installer with normal Windows security controls left enabled.
2. Launch Keepframe, create a new library in a disposable location, and confirm the initial empty library opens normally.
3. Import disposable JPEG photographs and, where privately supplied, CR2/CR3, NEF and ARW photographs using Copy. Confirm the original files remain and managed copies, previews and review screens open.
4. Close Keepframe, relaunch it, reopen the same library, and confirm the imported photographs and triage decisions remain available.
5. Where an earlier beta is available, install the current installer over it and repeat the launch/reopen check.
6. Uninstall Keepframe. Confirm the application is removed while user-selected libraries and the original photographs inside them remain intact. Inspect the Windows Recycle Bin before deleting any test library manually.

Record any SmartScreen, install, launch, migration, import, recovery or uninstall anomaly as a release blocker. Do not mark this gate as passed until a tester has completed and documented it.
