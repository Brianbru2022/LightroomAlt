import { useEffect, useRef, type MouseEvent } from "react";
import type { Asset } from "../types";

type Props={assets:Asset[];selectedIds:ReadonlySet<string>;activeId:string|null;onSelect:(asset:Asset,event?:Pick<MouseEvent,"ctrlKey"|"metaKey"|"shiftKey">)=>void;hasMore?:boolean;onLoadMore?:()=>void};

export function LibraryFilmstrip({assets,selectedIds,activeId,onSelect,hasMore=false,onLoadMore}:Props){
  const strip=useRef<HTMLDivElement>(null);
  useEffect(()=>{strip.current?.querySelector<HTMLElement>(`[data-filmstrip-id="${CSS.escape(activeId??"")}"]`)?.scrollIntoView?.({block:"nearest",inline:"nearest"});},[activeId]);
  return <div ref={strip} className="library-filmstrip" role="listbox" aria-label="Current result filmstrip" aria-multiselectable="true" onScroll={event=>{const node=event.currentTarget;if(hasMore&&onLoadMore&&node.scrollWidth-node.scrollLeft-node.clientWidth<480)onLoadMore();}}>
    {assets.map(asset=><button type="button" key={asset.id} data-filmstrip-id={asset.id} role="option" aria-selected={selectedIds.has(asset.id)} aria-current={activeId===asset.id?"true":undefined} className={`${selectedIds.has(asset.id)?"selected":""} ${activeId===asset.id?"active":""}`} onClick={event=>onSelect(asset,event)} title={`${asset.filename} · ${asset.rating} stars · ${asset.decision}`}><img src={asset.thumbnailUrl} alt="" loading="lazy"/><span className={`mini-state ${asset.decision}`}/><small>{asset.rating?`${asset.rating}★`:"—"}</small></button>)}
  </div>;
}
