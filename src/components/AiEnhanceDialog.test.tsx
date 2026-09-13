import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../lib/bridge";
import { demoAssets } from "../lib/demo";
import type { AiEnhancementHealth, AiEnhancementPreview } from "../types";
import { AiEnhanceDialog } from "./AiEnhanceDialog";

const health:AiEnhancementHealth={runtimeAvailable:true,busy:false,cudaAvailable:true,cudaFreeBytes:24e9,cudaTotalBytes:32e9,models:[{operation:"denoise",installed:true,loaded:false,provider:"Local",providerVersion:"1",model:"SCUNet",modelRevision:"pinned",modelSha256:"a".repeat(64),licence:"Apache-2.0",source:"upstream",approximateBytes:72e6,storagePath:"D:\\AI Models\\Keepframe",executionProvider:"CUDA",loadedMs:0,supportedScales:[1]},{operation:"super_resolution",installed:true,loaded:false,provider:"Local",providerVersion:"1",model:"Real-ESRGAN",modelRevision:"pinned",modelSha256:"b".repeat(64),licence:"BSD-3-Clause",source:"upstream",approximateBytes:67e6,storagePath:"D:\\AI Models\\Keepframe",executionProvider:"CUDA",loadedMs:0,supportedScales:[2,4]}]};
const preview:AiEnhancementPreview={previewId:"p",beforePath:"before",afterPath:"after",beforeUrl:"before.png",afterUrl:"after.png",width:400,height:300,provenance:{operation:"denoise",provider:"Local",providerVersion:"1",model:"SCUNet",modelRevision:"pinned",modelSha256:"a".repeat(64),executionProvider:"CUDA",scale:1,tileSize:512,overlap:32,tilePeakBytes:100,timings:{loadMs:1,preprocessMs:2,inferenceMs:3,postprocessMs:4,writeValidateMs:5,totalMs:15}}};

afterEach(()=>vi.restoreAllMocks());
describe("AI Enhance review",()=>{
  it("requires a preview before full Apply and keeps preview separate from acceptance",async()=>{
    vi.spyOn(api,"aiEnhancementHealth").mockResolvedValue(health);vi.spyOn(api,"aiDerivative").mockResolvedValue(null);vi.spyOn(api,"onAiEnhancementProgress").mockResolvedValue(()=>undefined);vi.spyOn(api,"previewAiEnhancement").mockResolvedValue(preview);const apply=vi.spyOn(api,"applyAiEnhancement");
    render(<AiEnhanceDialog asset={demoAssets[0]} onClose={vi.fn()} onChanged={vi.fn()} onOpenParent={vi.fn()}/>);
    expect(await screen.findByText("SCUNet")).toBeTruthy();expect(screen.getByRole("button",{name:"Apply full enhancement"})).toBeDisabled();
    fireEvent.click(screen.getByRole("button",{name:"Preview crop"}));await screen.findByAltText("AI enhancement preview");expect(apply).not.toHaveBeenCalled();expect(screen.getByRole("button",{name:"Apply full enhancement"})).not.toBeDisabled();
  });
  it("switches Super Resolution between exact 2x and 4x choices",async()=>{
    vi.spyOn(api,"aiEnhancementHealth").mockResolvedValue(health);vi.spyOn(api,"aiDerivative").mockResolvedValue(null);vi.spyOn(api,"onAiEnhancementProgress").mockResolvedValue(()=>undefined);
    render(<AiEnhanceDialog asset={demoAssets[0]} onClose={vi.fn()} onChanged={vi.fn()} onOpenParent={vi.fn()}/>);fireEvent.click(await screen.findByRole("button",{name:"Super Resolution"}));const select=screen.getByRole("combobox",{name:"Output scale"});expect(select).toHaveValue("4");fireEvent.change(select,{target:{value:"2"}});expect(select).toHaveValue("2");await waitFor(()=>expect(screen.getByText(/capped at 1024/)).toBeTruthy());
  });
});
