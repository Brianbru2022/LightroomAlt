import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SemanticDiscovery } from "./SemanticDiscovery";
import { api } from "../lib/bridge";
import { demoAssets } from "../lib/demo";

describe("Milestone 14 semantic discovery",()=>{
  beforeEach(()=>{vi.restoreAllMocks();window.history.replaceState({},"","/?semanticDemo=1");api.resetBrowserSession();});
  it("keeps search local, collapses source results and exposes explanations",async()=>{
    const onResults=vi.fn();render(<SemanticDiscovery active={demoAssets[0]} filter={{decision:"all",search:""}} selectedIds={new Set([demoAssets[0].id])} onResults={onResults} onClear={()=>{}} onLayout={()=>{}} onChanged={()=>{}} onNotice={()=>{}}/>);
    await screen.findAllByText(/Semantic index ready/);fireEvent.change(screen.getByLabelText("Semantic search"),{target:{value:"building at night"}});fireEvent.click(screen.getByRole("button",{name:"Search locally"}));
    await waitFor(()=>expect(onResults).toHaveBeenCalled());expect((await screen.findAllByText(/Local source-image semantic similarity/)).length).toBeGreaterThan(0);
  });
  it("requires an explicit action before a suggestion becomes a normal stack",async()=>{
    const createStack=vi.spyOn(api,"createStack");const onChanged=vi.fn();render(<SemanticDiscovery active={demoAssets[0]} filter={{decision:"all",search:""}} selectedIds={new Set()} onResults={()=>{}} onClear={()=>{}} onLayout={()=>{}} onChanged={onChanged} onNotice={()=>{}}/>);
    fireEvent.click(await screen.findByRole("button",{name:/Suggestions/}));expect(await screen.findByText(/Capture times within three seconds/)).toBeInTheDocument();expect(createStack).not.toHaveBeenCalled();expect(onChanged).not.toHaveBeenCalled();fireEvent.click(screen.getByRole("button",{name:"Accept as stack"}));await waitFor(()=>expect(createStack).toHaveBeenCalledOnce());expect(onChanged).toHaveBeenCalledOnce();
  });
  it("discards a stale completion after Clear",async()=>{
    const onResults=vi.fn();render(<SemanticDiscovery active={demoAssets[0]} filter={{decision:"all",search:""}} selectedIds={new Set()} onResults={onResults} onClear={()=>{}} onLayout={()=>{}} onChanged={()=>{}} onNotice={()=>{}}/>);
    await screen.findAllByText(/Semantic index ready/);fireEvent.change(screen.getByLabelText("Semantic search"),{target:{value:"sunset"}});fireEvent.click(screen.getByRole("button",{name:"Search locally"}));fireEvent.click(screen.getByRole("button",{name:/Clear/}));await new Promise(resolve=>setTimeout(resolve,180));expect(onResults).not.toHaveBeenCalled();
  });
  it("previews allow-listed Smart rules before explicit save",async()=>{
    const onChanged=vi.fn();render(<SemanticDiscovery active={demoAssets[0]} filter={{decision:"all",search:""}} selectedIds={new Set()} onResults={()=>{}} onClear={()=>{}} onLayout={()=>{}} onChanged={onChanged} onNotice={()=>{}}/>);
    fireEvent.click(await screen.findByRole("button",{name:/Smart proposal/}));fireEvent.click(screen.getByRole("button",{name:"Propose rules"}));expect(await screen.findByText(/Deterministic local parser/)).toBeInTheDocument();expect(onChanged).not.toHaveBeenCalled();fireEvent.click(screen.getByRole("button",{name:"Save Smart Collection explicitly"}));await waitFor(()=>expect(onChanged).toHaveBeenCalledOnce());
  });
});
