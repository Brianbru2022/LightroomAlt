# Release gates

Automated checks do not constitute commercial or public-release approval.

## Closed beta gate

- Clean-checkout validation and native disposable-profile smoke test pass.
- Installer install/upgrade/uninstall is tested on a clean Windows VM; uninstall preserves external libraries and models.
- A real disposable 2,000-photo run covers RAW/JPEG pairs, duplicates, corrupt files and interruption.
- Offline catalogue use and absent AI services are tested end to end.
- Zero unresolved critical data-safety or security/privacy defects.
- Unsigned installer is limited to named testers with checksum and SmartScreen instructions.

## Public free beta gate

- Final product name clears UKIPO, EUIPO, USPTO, domain and storefront searches with appropriate legal advice.
- Real 1,000-, 5,000- and 10,000-photo benchmarks meet recorded responsiveness budgets.
- Public installer is signed; update and rollback are tested across every released schema.
- Privacy policy, EULA, support route, AI disclosure, system requirements and third-party notices receive human review.
- Map tile use has an identifiable policy-compliant production configuration.
- Closed-beta high/critical defects are resolved and regression-tested.

## Paid Microsoft Store gate

- Final MSIX identity is reserved only after naming clearance.
- Windows App Certification Kit passes.
- Store licensing/trial never restricts library safety, ownership or export.
- Recovery tools have succeeded in realistic failure tests; migrations cover every beta version.
- Retention, repeat unscripted use, willingness to pay and one-maintainer support load have been evaluated.

Itch.io, Microsoft Store, Steam, signing, legal clearance and tester outcomes are external/human gates and must never be marked complete by repository tests alone.
