import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { demoAssets } from "../lib/demo";
import { LibraryProductivityView } from "./LibraryProductivityView";

describe("Milestone 12 Library productivity",()=>{
  it("supports desktop multi-selection, all-result selection, batch rating and Compare/Survey shortcuts",async()=>{
    render(<App/>);
    const first=await screen.findByRole("button",{name:"DSC_1842.NEF, keep"});
    const second=screen.getByRole("button",{name:"IMG_7721.CR3, undecided"});
    const fourth=screen.getByRole("button",{name:"DSC_0904.ARW, discard"});
    fireEvent.click(first);
    fireEvent.click(second,{ctrlKey:true});
    expect(screen.getByText(/2 selected \(2 visible\)/)).toBeInTheDocument();
    fireEvent.click(fourth,{shiftKey:true});
    expect(screen.getByText(/3 selected \(3 visible\)/)).toBeInTheDocument();
    fireEvent.keyDown(window,{key:"a",ctrlKey:true});
    expect(screen.getByText(/6 selected \(6 visible\)/)).toBeInTheDocument();
    fireEvent.keyDown(window,{key:"5"});
    await waitFor(()=>expect(document.querySelectorAll('[title^="5 stars"]').length).toBe(6));
    fireEvent.keyDown(window,{key:"c"});
    expect(await screen.findByText("Reference")).toBeInTheDocument();
    expect(screen.getByText("Candidate")).toBeInTheDocument();
    fireEvent.keyDown(window,{key:"n"});
    expect(await screen.findByText("Surveying 6 photographs.")).toBeInTheDocument();
  });

  it("does not fire rating or flag shortcuts from an editable field",async()=>{
    render(<App/>);
    const card=await screen.findByRole("button",{name:"DSC_1842.NEF, keep"});
    const search=screen.getByRole("textbox",{name:"Search photographs"});
    fireEvent.focus(search);fireEvent.keyDown(search,{key:"x"});fireEvent.keyDown(search,{key:"1"});
    expect(card).toHaveAttribute("aria-label","DSC_1842.NEF, keep");
    expect(card).toHaveAttribute("title","4 stars · active photograph");
  });

  it("auto-advances a single cull and keeps risky Sync categories off by default",async()=>{
    render(<App/>);
    const first=await screen.findByRole("button",{name:"DSC_1842.NEF, keep"});
    fireEvent.click(screen.getByLabelText("Auto Advance"));
    fireEvent.click(first);
    fireEvent.keyDown(window,{key:"x"});
    await waitFor(()=>expect(screen.getByRole("button",{name:"IMG_7721.CR3, undecided"})).toHaveAttribute("title","0 stars · active photograph"));
    expect(screen.getByRole("button",{name:"DSC_1842.NEF, discard"})).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button",{name:"P1060420.RW2, keep"}),{ctrlKey:true});
    fireEvent.click(screen.getByRole("button",{name:"Sync settings"}));
    const checks=[...document.querySelectorAll<HTMLInputElement>('.sync-options input[type="checkbox"]')].map(input=>input.checked);
    expect(checks).toEqual([true,true,true,true,false,false,false]);
    expect(screen.getByText(/does not rerun AI/)).toBeInTheDocument();
  });

  it("records Compare switching and a 12-image Survey render",()=>{
    const assets=[...demoAssets,...demoAssets.map((asset,index)=>({...asset,id:`survey-${index}`,filename:`Survey ${index}.jpg`}))];
    const ids=new Set(assets.map(asset=>asset.id));
    const summary={requested:assets.length,changed:assets.length,failed:0,cancelled:0};
    const common={assets,total:assets.length,visibleSelectedCount:assets.length,loading:false,hasMore:false,onLoadMore:vi.fn(),filter:{decision:"all" as const,search:""},onFilter:vi.fn(),active:assets[0],activeId:assets[0].id,selectedIds:ids,onLayout:vi.fn(),onSelect:vi.fn(),onSetActive:vi.fn(),onRemove:vi.fn(),onClear:vi.fn(),onOpenDevelop:vi.fn(),onMetadata:vi.fn(async()=>summary),onReviewAsset:vi.fn(async()=>undefined),onSync:vi.fn(async()=>summary),onPreset:vi.fn(async()=>summary),onBatchAuto:vi.fn(async()=>({...summary,analyzable:assets.length,lowConfidenceWhiteBalance:0,skipped:0})),onExport:vi.fn(),autoAdvance:false,onAutoAdvance:vi.fn()};
    const compareStarted=performance.now();
    const view=render(<LibraryProductivityView {...common} layout="compare"/>);
    const compareRenderMs=performance.now()-compareStarted;
    const switchStarted=performance.now();
    fireEvent.click(screen.getByRole("button",{name:"Next candidate"}));
    const compareSwitchMs=performance.now()-switchStarted;
    const surveyStarted=performance.now();
    view.rerender(<LibraryProductivityView {...common} layout="survey"/>);
    const surveyRenderMs=performance.now()-surveyStarted;
    expect(screen.getByText("Surveying 12 photographs.")).toBeInTheDocument();
    expect(screen.getAllByRole("button",{name:/Remove .* from Survey/})).toHaveLength(12);
    expect(compareRenderMs).toBeLessThan(1_000);
    expect(compareSwitchMs).toBeLessThan(250);
    expect(surveyRenderMs).toBeLessThan(1_000);
    console.info(`M12_UI_BENCHMARK compare_render_ms=${compareRenderMs.toFixed(3)} compare_switch_ms=${compareSwitchMs.toFixed(3)} survey_12_render_ms=${surveyRenderMs.toFixed(3)}`);
  });
});
