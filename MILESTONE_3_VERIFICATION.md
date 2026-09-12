# Milestone 3 external verification checklist

Milestone 3's automated tests exercise the local hand-off, result validation,
version provenance and unavailable-service paths. The following checks require
a human and a disposable photograph; they are not claims that a provider has
been exercised.

## Real local Qwen edit

1. Use a disposable library and copy a non-sensitive JPEG or PNG into it.
2. Start the separately installed Qwen image-edit service on a loopback-only
   address, normally `http://127.0.0.1:7868`, with its Qwen image-edit model
   installed and exposed by `/api/status`.
3. In **Settings & Health**, confirm the local editor reads **available** and
   identifies the expected model. If it does not, use **Refresh status**; do
   not submit an edit while it is unavailable or busy.
4. In **AI Workshop**, choose one restrained action, start a local edit and
   wait for a candidate result.
5. Compare the candidate with the protected original. Confirm the original
   hash/path is unchanged and that the candidate records `local-qwen`, prompt,
   recipe, source hash and output hash.
6. Accept the candidate, close and reopen Keepframe, then confirm the accepted
   version and job attempt history persist.

## Manual ChatGPT or Gemini hand-off

1. Use a disposable image and choose one action in **AI Workshop**.
2. Select **ChatGPT hand-off** or **Gemini hand-off**, then choose **Prepare
   image and instruction**. Confirm Keepframe creates a local sRGB PNG, prompt
   file and `waiting_external` job.
3. Upload the PNG and instruction to the provider yourself. Keepframe never
   uploads the image or calls either provider API.
4. Save the returned image locally and choose **Import returned image**.
5. Confirm the imported result becomes a candidate version with the provider,
   prompt, structured recipe and hashes. Confirm the original remains unchanged,
   then accept/reject the candidate as appropriate.

## Packaged Windows app

1. Install the current authorised test build in a disposable profile.
2. Open a library and AI Workshop, then verify the provider-health text and
   loopback-only local service setting.
3. Prepare a manual external hand-off and import a known valid returned image.
4. Close and reopen the app. Confirm the `waiting_external`/completed job and
   AI-derived version persist, and the protected original is still selectable.

Do not upload private photographs to an external provider unless you have made
that decision explicitly and understand that provider's terms.
