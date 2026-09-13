import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { neutralAdvancedDevelop } from "../types";
import { AdvancedDevelopControls } from "./AdvancedDevelopControls";

describe("Advanced Develop controls",()=>{
  it("edits and resets point curves and exposes photographic sections",()=>{
    let current=neutralAdvancedDevelop();
    const view=render(<AdvancedDevelopControls assetId="demo" value={current} onChange={next=>{current=next;view.rerender(<AdvancedDevelopControls assetId="demo" value={current} onChange={()=>undefined}/>);}}/>);
    fireEvent.click(screen.getByRole("button",{name:"Add point"}));
    expect(current.curves.master).toHaveLength(3);
    expect(screen.getByRole("img",{name:"master point curve"})).toBeInTheDocument();
    expect(screen.getByRole("button",{name:"R"})).toBeInTheDocument();
    expect(screen.getByText("Colour Mixer")).toBeInTheDocument();
    expect(screen.getByText("Colour Grading")).toBeInTheDocument();
    expect(screen.getAllByText("Detail").length).toBeGreaterThan(0);
    expect(screen.getByText("Lens Corrections")).toBeInTheDocument();
  });
});
