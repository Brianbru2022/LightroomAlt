import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { demoAssets } from "../lib/demo";
import { api } from "../lib/bridge";
import { ExportDialog } from "./ExportDialog";

afterEach(()=>vi.restoreAllMocks());

describe("professional export dialog",()=>{
  it("shows the full workflow, safe defaults, estimates, and a dedicated batch queue",async()=>{
    render(<ExportDialog assets={demoAssets.slice(0,2)} selected={demoAssets[0]} onClose={vi.fn()}/>);
    expect(await screen.findByRole("heading",{name:"Export photographs"})).toBeTruthy();
    for(const heading of ["Export preset","Queue","File settings","Image sizing","Metadata","Filename & destination","Output summary"])expect(screen.getAllByText(heading).length).toBeGreaterThan(0);
    expect(screen.getByRole("combobox",{name:"Colour space"})).toHaveValue("srgb");
    expect(screen.getByLabelText(/do not enlarge/i)).toBeChecked();
    expect(screen.getByLabelText(/location \/ gps/i)).not.toBeChecked();
    const queue=screen.getByText("Queue").closest("fieldset")!;fireEvent.click(within(queue).getAllByRole("checkbox")[1]);
    expect(screen.getByRole("button",{name:"Export 2 photographs"})).toBeTruthy();
    expect(screen.getByText(/MP$/)).toBeTruthy();
  });

  it("keeps format semantics honest and presents a structured completion report",async()=>{
    const start=vi.spyOn(api,"startExportBatch");
    render(<ExportDialog assets={[demoAssets[0]]} selected={demoAssets[0]} onClose={vi.fn()}/>);await screen.findByRole("option",{name:/Full-size JPEG/});
    fireEvent.change(screen.getByLabelText("Format"),{target:{value:"tiff"}});
    expect(screen.getByText(/TIFF is 8-bit RGB, uncompressed/)).toBeTruthy();
    expect(screen.queryByLabelText("JPEG quality")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:"Export 1 photograph"}));
    expect(await screen.findByText("1 complete")).toBeTruthy();
    await waitFor(()=>expect(start).toHaveBeenCalled());
    expect(screen.getByRole("button",{name:"Open folder"})).toBeTruthy();
    expect(screen.getByText(/0 failed · 0 skipped · 0 cancelled/)).toBeTruthy();
  });
});
