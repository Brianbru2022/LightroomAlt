import { describe, expect, it } from "vitest";
import { demoAssets, makeRecipe, renderPrompts } from "./demo";

describe("provider-neutral edit recipes", () => {
  it("keeps identity and source authority in every provider prompt", () => {
    const recipe = makeRecipe(demoAssets[5], "scratch_repair");
    const prompts = renderPrompts(recipe);
    expect(recipe.preserve).toContain("identity_faces");
    expect(prompts.local).toContain("Do not reshape faces");
    expect(prompts.chatgpt).toContain("source as authoritative");
    expect(prompts.gemini).toContain("no unrequested generative changes");
  });

  it("uses individual observations for different photographs", () => {
    const archive = makeRecipe(demoAssets[5]);
    const modern = makeRecipe(demoAssets[0]);
    expect(archive.observations).not.toEqual(modern.observations);
  });
});
