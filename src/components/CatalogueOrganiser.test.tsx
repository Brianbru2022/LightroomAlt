import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CatalogueOrganiser } from "./CatalogueOrganiser";
import { api } from "../lib/bridge";
import { demoAssets } from "../lib/demo";

describe("Milestone 13 catalogue organisation",()=>{
  beforeEach(()=>api.resetBrowserSession());

  it("creates item-specific Collections, a Smart Collection and a source-sharing version",async()=>{
    const onFilter=vi.fn();const onChanged=vi.fn();const onNotice=vi.fn();
    render(<CatalogueOrganiser filter={{decision:"all",search:""}} onFilter={onFilter} selectedIds={new Set([demoAssets[0].id])} active={demoAssets[0]} onChanged={onChanged} onNotice={onNotice}/>);
    await screen.findByText("All Photographs");fireEvent.click(screen.getByRole("button",{name:"New Collection"}));fireEvent.change(screen.getByRole("dialog",{name:"Collection name"}).querySelector("input")!,{target:{value:"Portfolio"}});fireEvent.click(screen.getByRole("button",{name:"Create"}));expect(await screen.findByText("Portfolio")).toBeInTheDocument();fireEvent.click(screen.getByTitle("Add selected"));await waitFor(()=>expect(screen.getByText("1")).toBeInTheDocument());fireEvent.click(screen.getByText("Portfolio"));expect(onFilter).toHaveBeenCalledWith(expect.objectContaining({collectionId:expect.any(String)}));
    fireEvent.click(screen.getByRole("button",{name:"From current"}));fireEvent.change(screen.getByRole("dialog",{name:"Version name"}).querySelector("input")!,{target:{value:"Print"}});fireEvent.click(screen.getByRole("button",{name:"Create"}));expect(await screen.findByText("Print")).toBeInTheDocument();expect(await api.catalogueVersions(demoAssets[0].id)).toHaveLength(2);
    fireEvent.click(screen.getByRole("button",{name:"+ Smart Collection"}));expect(screen.getByRole("dialog",{name:"Smart Collection editor"})).toBeInTheDocument();fireEvent.change(screen.getByLabelText("Name"),{target:{value:"Four-star work"}});fireEvent.click(screen.getByRole("button",{name:"Save Smart Collection"}));expect(await screen.findByRole("button",{name:/^◆ Four-star work/})).toBeInTheDocument();expect(onNotice).toHaveBeenCalledWith(expect.stringContaining("Smart Collection"));
  });
});
