# Milestone 15: Advanced Develop, Detail and Optical Corrections

## Scope and recipe

Milestone 15 extends the non-destructive Develop recipe from version 2 to version 3. Version 1 and 2 JSON is accepted, validated and migrated in memory with exact neutral defaults for every new field. The catalogue stores the complete recipe JSON; no original pixels or original EXIF are rewritten. Catalogue schema 14 adds only nullable source facts (`focal_length`, `aperture` and `iso`) needed for strict optical-profile identification. Existing rows remain null and valid.

The new `advanced` recipe object contains master/red/green/blue point curves, eight Colour Mixer bands, three-way grading, capture-detail controls and optical settings including the selected profile ID and pinned data revision. Virtual versions retain independent recipes while sharing immutable source facts. Portable catalogue JSON therefore retains all advanced settings and profile identity without embedding an external profile database.

## Deliberate render order

The shared native renderer uses this order:

1. full or browsing source decode and existing orientation handling;
2. optical distortion, lateral chromatic aberration and lens-vignette correction;
3. existing Basic white balance, tone and presence processing;
4. master and RGB-channel point curves;
5. HSL/Colour Mixer;
6. three-way colour grading;
7. luminance NR, chroma NR and capture sharpening;
8. ordered local and semantic masks, remapped through the optical transform;
9. crop, straighten, rotation and flips;
10. export resize;
11. Milestone 11 output sharpening;
12. JPEG, PNG or TIFF encoding.

Presence remains inside the established Basic stage to preserve Milestones 6–10 compatibility. Preview and full-resolution export call the same `render_develop_recipe_cached` implementation. A Fit preview has fewer source samples than a 100% view, so fine detail must be judged near 100%; this is stated in the UI.

## Curves

Each point curve stores 2–16 normalised `(x,y)` points. Endpoints at x=0 and x=1 are mandatory. Validation sorts by x, rejects non-finite/out-of-range values and rejects points closer than 0.005 on x. Imported malformed recipes or presets fail closed.

Interpolation is shape-preserving cubic Hermite interpolation. Neighbouring secant slopes generate harmonic tangents, with a zero tangent across slope reversals, limiting overshoot and unintended inversion. The master curve maps luminance before true per-channel curves. The graph provides channel selection, direct double-click point creation, add/remove/reset actions and keyboard-accessible numeric input/output fields. The established four-region Shadows/Darks/Lights/Highlights controls remain as the parametric workflow; no second overlapping parametric implementation was added. The existing shared histogram sits immediately above the editor and follows the current preview URL, avoiding a separate circular histogram render.

## Colour Mixer and grading

The mixer uses Red, Orange, Yellow, Green, Aqua, Blue, Purple and Magenta. RGB is converted to HSL, adjusted, then converted back. Each band has a circular hue-distance weight with overlapping smoothstep shoulders; hue wraps at 0/360 degrees and adjacent bands blend without a hard seam. Hue, saturation and luminance have different operations. A targeted-on-image HSL tool is deferred because it would require reliable sampled edited pixels and gesture arbitration.

Grading applies shadow, midtone and highlight hues in luminance-weighted zones. Smoothstep overlap is controlled by Blending; Balance shifts both tonal transitions. Each wheel has precise hue/saturation sliders plus a visible swatch. There is no hidden global grade.

## Detail processing

Capture sharpening is an unsharp-mask-derived two-scale filter. Radius controls the broad blur, Detail blends a fine residual, Amount controls strength, and Masking thresholds a luminance edge map so smooth areas receive less sharpening. It is separate from post-resize export sharpening.

Luminance NR uses a blurred luminance target with an edge guard; Detail changes edge preservation and Contrast restores local luminance residual. Colour NR separates pixel luminance from per-channel chroma, smooths chroma only, and uses Detail and Smoothness independently. Algorithms use Rayon’s existing global pool and deterministic buffers. Moderate settings are intended for practical local clean-up, not state-of-the-art high-ISO reconstruction. No AI model is downloaded or used. ISO is displayed as source context but never silently changes a recipe.

RAW paths remain the established LibRaw/ExifTool browsing path. LightroomAlt adds no hidden RAW sharpening or denoise flags; visible advanced stages occur after the decoded image. Native qualification with actual supported high/low-ISO RAW files remains required because repository fixtures cannot establish camera-specific quality.

## Optical subsystem and profile policy

Profile identification, parameter storage and rendering are separate. The bundled data is a deliberately tiny transcription of exact calibration rows from the Lensfun project, licensed CC-BY-SA-3.0, pinned to git revision `12f5976ce30c024f98c420835125b9676ac07811`. Source rows are named in code and the project notice. No Adobe or other proprietary profile is used.

Two exact profile identities are currently bundled: Canon EOS 650D + Canon EF 50mm f/1.8 at 50 mm/f2.8, and Nikon D750 + Nikon AF Nikkor 50mm f/1.8D at 50 mm/f2.5. Automatic matching requires case-insensitive exact camera and lens strings plus focal length within 0.25 mm and aperture within 0.2 stop. A miss is reported, never fuzzily substituted. Manual profile selection stores both ID and revision. A well-formed stored identity that is no longer available remains loadable and visible, with its manual distortion/CA/vignette values preserved; the renderer does not substitute another profile. Malformed imported identities fail validation.

Distortion uses radial polynomial coefficients and supports barrel, pincushion and the bundled multi-term mapping. Lateral CA uses separate red and blue coordinate scales. Vignette correction uses the profile radial polynomial or a manual Amount/linear coefficient with a Midpoint fall-off. Bilinear sampling is used; outside-source samples are deliberate black pixels. `Constrain crop` is on by default and scales the sampling map inward, but never changes the user’s crop rectangle. Creative post-crop vignette and purple/green defringe are intentionally deferred and not conflated with optical correction.

## Masks and geometry

Masks remain authored in canonical source coordinates. Optical correction samples source coordinates for every output pixel; the same bilinear transform remaps manual, linear, radial and accepted semantic coverage before local adjustments. The browser overlay uses the corresponding radial mapping and inverse iteration before crop/rotation display transforms. Existing semantic coverage does not need AI re-analysis. Dedicated native image/mask equivalence tests cover the mapping; existing mask/crop/rotate/straighten/flip regressions remain in the full suite.

## Presets, Auto, Sync and history

Preset schema 2 adds explicit Curve, Colour Mixer, Colour Grading, Detail and Lens Corrections categories. Version 1 presets migrate with neutral advanced defaults; old presets cannot silently enable optics. Copy/Paste includes the full advanced global recipe, while masks remain opt-in. Reset All returns all fields to exact no-op; each panel provides a relevant section reset where meaningful.

Milestone 7 Auto still proposes Basic settings only. Sync offers advanced categories explicitly. `Auto Lens Profile` resolves strict source metadata independently for each destination and turns correction off on a miss. `Lens profile: Exact` deliberately copies the source profile and manual values. Neither lens option is selected by default. Slider changes use the existing grouped history path; discrete profile changes form a single state change.

## Caches, precision and memory

Decode cache identity is unchanged. The bounded global intermediate key now depends on the Basic colour identity and the entire validated advanced object; masks remain independently cached in canonical form and are cheaply remapped for the active optical settings. This avoids invalidating decode and avoids retaining a large coordinate-map cache. Detail filters transiently hold approximately two or three RGB8 scratch images (about 2.1–3.1 MiB at 720×480 and 22–33 MiB at 2400×1600). Lens remapping allocates one RGB8 result and no retained map. Existing decode/intermediate/mask/semantic bounds remain 96/96/64/64 MiB.

RGB8 is retained. It preserves existing deterministic/export behaviour and bounds memory, but repeated strong curves or grading can quantise smooth gradients. An RGB16/float conversion is deferred until controlled visual and performance evidence justifies the cache and compatibility cost.

Run `scripts/benchmark-advanced-develop.ps1` for per-stage 720×480 and combined 2400×1600 timings plus the Milestone 10 default-control renderer regression. Targets are reference thresholds, not claims across every PC. The benchmark output and machine context should be kept with release evidence.

The 2026-09-13 recorded local release run measured the following cold/warm stage times in milliseconds: master curve 4.962/3.608; RGB channel curves 3.255/3.563; HSL 3.378/3.250; grading 2.780/2.820; sharpening 13.787/13.706; luminance NR 9.420/8.964; colour NR 10.568/11.261; distortion 4.338/4.178; lateral CA 4.067/4.758; vignette 4.032/4.750. The combined 1024×768 detail region measured 73.401/74.552 ms, and the full advanced 2400×1600 render measured 435.410/425.688 ms. These synthetic timings exclude decode and encode. The same evidence file records the all-default Milestone 10 path at 71.489 ms cold/1.088 ms warm for 720×480 and 294.793 ms for a 2400×1600 render. No larger real-camera full-resolution fixture was available, so that qualification remains external.

## Interoperability and privacy boundaries

The conservative XMP subset remains metadata/triage-oriented. Advanced Develop settings and Lensfun profile IDs are preserved in portable catalogue JSON, not presented as interoperable Adobe XMP. Profile availability must be checked after moving a catalogue; revision mismatch is explicit. All advanced rendering and metadata matching is local and adds no network access, analytics or model download.

## Known limitations

- The bundled Lensfun subset is intentionally very small; most photographs use manual correction until a reviewed, licensed profile-ingestion/update policy is added.
- Focus distance is not currently extracted or matched.
- A targeted HSL tool, sharpening-mask preview, detail navigator, defringe and creative vignette are deferred.
- Fit previews approximate fine detail. Native 100% inspection is required for halos, smearing, CA and real lens geometry.
- Bilinear remapping favours predictable speed; very large corrections may benefit from a higher-order sampler later.
- Repository tests use deterministic synthetic and rendered fixtures. Legal, non-private real JPEG/TIFF/RAW qualification and native colour-managed display acceptance remain external.

## Disposable native acceptance checklist

1. Import/open JPEG; 2. import supported RAW; 3. add S-curve; 4. edit red curve; 5. restart; 6. adjust Orange HSL; 7. adjust Blue HSL; 8. grade all tonal zones; 9. inspect Before/After; 10. inspect sharpening at 100%; 11. increase Masking; 12. test luminance NR; 13. test colour NR; 14. compare edge/detail retention; 15. open wide-angle photo; 16. enable profile; 17. verify identity/status; 18. toggle correction; 19. adjust distortion; 20. inspect CA; 21. apply CA correction; 22. inspect lens-vignette correction; 23. crop; 24. create brush before/after correction; 25. verify attachment; 26. verify accepted semantic mask; 27. create virtual version; 28. give sibling a different advanced recipe; 29. Compare versions; 30. Sync HSL only; 31. test mixed-lens Auto versus Exact Sync; 32. apply old/new preset; 33. export JPEG; 34. export PNG; 35. export TIFF; 36. compare visual parity; 37. inspect metadata/profile behaviour; 38. hash originals; 39. verify unchanged; 40. observe response time and memory.
