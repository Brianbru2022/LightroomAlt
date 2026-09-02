# Invitation-only beta procedure

Use this unsigned build only with 5–10 named testers and disposable copies of 500–2,000 photographs. Send each tester the installer SHA-256 and explain the expected SmartScreen warning. Do not publish a general download.

Each tester should complete: Copy import, restart, timeline and tag browsing, Keep/Discard triage, Trash/Restore, map browsing and manual GPS placement, recipe generation, external PNG/prompt export, returned-edit import and version comparison. Optional local GPU editing is restricted to technically confident testers with compatible hardware.

Stop testing immediately for any case where neither the original source nor a verified managed copy exists. Treat every uncertainty about an original’s location as a priority defect. Ask testers to use Settings & Library Health to export diagnostics explicitly; do not collect their library or prompts automatically.

Record Windows version, display scaling, collection composition, timings, failures, whether help was required and whether the tester returned for at least three sessions. Closed beta succeeds only when the criteria in `RELEASE_GATES.md` are evidenced.

## Installer verification and SmartScreen

Before running the invitation-only installer, compare its SHA-256 with `release\SHA256SUMS.txt`:

```powershell
Get-FileHash -Algorithm SHA256 .\Keepframe_0.2.0-beta.1_x64-setup.exe
```

This beta is unsigned, so Windows may show a Microsoft Defender SmartScreen warning. Only named testers who received the checksum directly should choose **More info**, verify the publisher is unknown as expected, verify the filename/checksum, and then choose **Run anyway**. Do not disable SmartScreen or other Windows security controls. Public builds must be signed.
